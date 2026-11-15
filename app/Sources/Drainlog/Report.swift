import Foundation

// What the report says about a night, worked out from its rows. No SwiftUI here.

enum Fmt {
    /// "45 s", "52 min", "7 h 14 min".
    static func duration(_ seconds: Int) -> String {
        switch seconds {
        case ..<60: return "\(seconds) s"
        case ..<3600: return "\((seconds + 30) / 60) min"
        default:
            let minutes = (seconds + 30) / 60
            return minutes % 60 == 0 ? "\(minutes / 60) h" : "\(minutes / 60) h \(minutes % 60) min"
        }
    }

    /// "520 mAh", or "≈105 mAh" for an estimate.
    static func mah(_ mah: Int, approximate: Bool = false) -> String {
        "\(approximate ? "≈" : "")\(mah.formatted()) mAh"
    }

    /// The same for VoiceOver.
    static func spokenMah(_ mah: Int, approximate: Bool = false) -> String {
        "\(approximate ? "about " : "")\(mah.formatted()) milliamp-hours"
    }

    /// A part's share of the night's drop: "20%".
    static func share(_ mah: Int, of drop: Int) -> String {
        drop > 0 ? "\(Int((Double(mah) * 100 / Double(drop)).rounded()))%" : "–"
    }

    static func plural(_ n: Int, _ word: String) -> String {
        "\(n.formatted()) \(word)\(n == 1 ? "" : "s")"
    }

    static func capitalized(_ s: String) -> String {
        s.prefix(1).uppercased() + s.dropFirst()
    }

    /// A fix as a line of its own: commands as written, sentences capitalized.
    static func fix(_ text: String) -> String {
        isCommand(text) ? text : capitalized(text)
    }

    static func isCommand(_ text: String) -> Bool { text.hasPrefix("sudo ") }

    static func date(_ ts: Int64) -> Date { Date(timeIntervalSince1970: TimeInterval(ts)) }

    /// "23:10", in the user's 12/24-hour setting.
    static func clock(_ ts: Int64) -> String {
        date(ts).formatted(Date.FormatStyle(locale: locale).hour().minute())
    }

    /// "Wed 12 – Thu 13 Nov".
    static func range(_ night: Night) -> String {
        (date(night.id)..<date(night.end)).formatted(
            Date.IntervalFormatStyle(locale: locale).weekday(.abbreviated).day().month(.abbreviated))
    }

    /// "13 Nov".
    static func day(_ date: Date) -> String {
        date.formatted(Date.FormatStyle(locale: locale).day().month(.abbreviated))
    }

    /// The app's language (its copy is English) with the user's region and clock settings, so
    /// dates never come out in another language than the sentences around them.
    static let locale: Locale = {
        var parts = Locale.Components(locale: .current)
        var language = Locale.Language.Components(identifier: Bundle.main.preferredLocalizations.first ?? "en")
        language.region = Locale.current.region
        parts.languageComponents = language
        return Locale(components: parts)
    }()
}

extension Night {
    /// The headline after "You lost …% overnight.": "Mostly Power Nap / maintenance wakes."
    var verdict: String {
        headline.range(of: "overnight. ").map { String(headline[$0.upperBound...]) } ?? headline
    }

    /// The bucket the report names, if it names one.
    func top(_ causes: [Cause]) -> Cause? {
        causes.first { $0.key == topCause }
    }

    /// "Lid closed at 23:10 on 82%. Opened at 07:20 on 68%."
    func sentence(_ intervals: [Interval]) -> String {
        let lid = intervals.first { $0.start == id }?.reason == "Clamshell Sleep"
        let opened = intervals.last { $0.start == end && $0.state == .awake }
            .map { $0.reason.split(separator: " ").contains("lid") } ?? false
        return "\(lid ? "Lid closed" : "Went to sleep") at \(Fmt.clock(id)) on \(startPct)%. "
            + "\(opened ? "Opened" : "Woke") at \(Fmt.clock(end)) on \(endPct)%."
    }
}

/// Seconds per state, for the chart's legend and its VoiceOver summary.
struct NightTotals: Equatable {
    var asleep = 0, darkWake = 0, awake = 0
    /// DarkWakes the Mac woke into, not the ones it entered on its way to sleep.
    var wakes = 0

    init(_ night: Night, _ intervals: [Interval]) {
        for iv in intervals where iv.night == night.id {
            let seconds = Int(iv.end - iv.start)
            switch iv.state {
            case .asleep: asleep += seconds
            case .awake: awake += seconds
            case .darkwake:
                darkWake += seconds
                // Entered while going to sleep, the reason is the sleep reason ("Clamshell Sleep").
                if !iv.reason.hasSuffix("Sleep") { wakes += 1 }
            }
        }
    }
}

/// One ledger row.
struct LedgerLine: Identifiable, Hashable {
    enum Tag { case measured, estimated, unexplained, notMeasured }
    enum Kind { case plain, group, child, unexplained, total }

    let id: String
    let name: String
    let detail: String
    let mah: Int
    let approximate: Bool
    let tag: Tag
    let kind: Kind
}

enum Ledger {
    /// The rows that open on demand: baseline, DarkWakes grouped by wake reason, awake time,
    /// then whatever held the Mac awake.
    static func parts(_ night: Night, _ causes: [Cause]) -> [LedgerLine] {
        var out: [LedgerLine] = []
        if let base = causes.first(where: { $0.key == "baseline" }) {
            out.append(LedgerLine(
                id: base.key, name: "Baseline sleep",
                detail: night.calibrated
                    ? "\(Fmt.duration(base.seconds)) asleep, at your measured sleep rate"
                    : "Baseline not measured yet",
                mah: base.mah, approximate: true, tag: night.calibrated ? .estimated : .notMeasured,
                kind: .plain))
        }
        func line(_ c: Cause, _ name: String, _ detail: String, _ kind: LedgerLine.Kind) -> LedgerLine {
            LedgerLine(
                id: c.key, name: name, detail: detail, mah: c.mah, approximate: c.estimated,
                tag: c.estimated ? .estimated : .measured, kind: kind)
        }
        // The core lists attributed causes largest first; that order is kept within each part.
        let attributed = causes.filter { !["baseline", "unexplained"].contains($0.key) }
        let wakes = attributed.filter { $0.key != "awake" && !$0.key.hasPrefix("assertion:") }
        if wakes.count > 1 {
            let estimated = wakes.contains { $0.estimated }
            out.append(LedgerLine(
                id: "darkwakes", name: "DarkWakes",
                detail: "\(Fmt.plural(wakes.map(\.count).reduce(0, +), "wake")), "
                    + "\(Fmt.duration(wakes.map(\.seconds).reduce(0, +))), by wake reason",
                mah: wakes.map(\.mah).reduce(0, +), approximate: estimated,
                tag: estimated ? .estimated : .measured, kind: .group))
            out += wakes.map {
                line($0, Fmt.capitalized($0.label.replacingOccurrences(of: " wakes", with: "")),
                     Fmt.plural($0.count, "wake"), .child)
            }
        } else {
            out += wakes.map {
                line($0, Fmt.capitalized($0.label),
                     "\(Fmt.plural($0.count, "wake")), \(Fmt.duration($0.seconds))", .plain)
            }
        }
        for c in attributed where !wakes.contains(c) {
            out.append(c.key == "awake"
                ? line(c, "Awake", c.count > 0
                    ? "\(Fmt.duration(c.seconds)) in \(Fmt.plural(c.count, "wake"))"
                    : Fmt.duration(c.seconds), .plain)
                : line(c, Fmt.capitalized(c.label), Fmt.duration(c.seconds), .plain))
        }
        return out
    }

    /// Always shown: the measured drop minus every attributed part. May be negative.
    static func unexplained(_ night: Night, _ causes: [Cause]) -> LedgerLine {
        LedgerLine(
            id: "unexplained", name: "Unexplained",
            detail: night.calibrated
                ? "Total drop minus everything Drainlog can attribute"
                : "May include normal sleep drain until the baseline is measured",
            mah: causes.first { $0.key == "unexplained" }?.mah ?? 0, approximate: !night.measured,
            tag: .unexplained, kind: .unexplained)
    }

    static func total(_ night: Night) -> LedgerLine {
        let span = "\(Fmt.clock(night.id)) to \(Fmt.clock(night.end))"
        return LedgerLine(
            id: "total", name: "Total",
            detail: night.measured ? "Battery counter, \(span)" : "From Charge %, \(span)",
            mah: night.dropMah, approximate: !night.measured,
            tag: night.measured ? .measured : .estimated, kind: .total)
    }
}

/// One line of the incident timeline.
struct Incident: Identifiable, Hashable {
    let id: Int
    /// "23:10", or "All night" for a line about the whole night.
    let time: String
    let text: String
    /// Set on the "Fix: …" line: the cause whose fix and undo it shows.
    let fix: Cause?

    static func list(_ night: Night, _ causes: [Cause]) -> [Incident] {
        night.timeline.enumerated().map { i, item in
            let fix = item.text.hasPrefix("Fix: ") ? night.top(causes).flatMap { $0.fix.isEmpty ? nil : $0 } : nil
            return Incident(
                id: i, time: item.ts.map(Fmt.clock) ?? "All night", text: item.text, fix: fix)
        }
    }
}

/// Where in System Settings a fix can be made, for the fixes that have a place there.
func settingsPane(for cause: String) -> (title: String, url: URL)? {
    switch cause {
    case "powernap":
        ("Open Battery Settings", URL(string: "x-apple.systempreferences:com.apple.Battery-Settings.extension")!)
    case "bluetooth":
        ("Open Bluetooth Settings", URL(string: "x-apple.systempreferences:com.apple.BluetoothSettings")!)
    default: nil
    }
}
