import Foundation
import SQLite3
import Testing

@testable import Drainlog

/// A night from 23:00 to 07:00 (+08:00), 2026-11-03.
private let start: Int64 = 1_793_718_000
private let end: Int64 = start + 8 * 3600

/// A database with drainlogd's real schema, read from core/src/store.rs so the two can't drift.
private func makeDatabase(_ rows: String) throws -> URL {
    let core = URL(filePath: #filePath).deletingLastPathComponent().appending(path: "../../../core/src/store.rs")
    let rust = try String(contentsOf: core, encoding: .utf8)
    let schema = try #require(rust.components(separatedBy: "r#\"").dropFirst().first?.components(separatedBy: "\"#;").first)
    let url = URL.temporaryDirectory.appending(path: "drainlog-test-\(UUID().uuidString).db")
    var db: OpaquePointer?
    #expect(sqlite3_open(url.path, &db) == SQLITE_OK)
    defer { sqlite3_close(db) }
    for sql in [schema, rows] {
        var message: UnsafeMutablePointer<CChar>?
        let rc = sqlite3_exec(db, sql, nil, nil, &message)
        #expect(rc == SQLITE_OK, "\(message.map { String(cString: $0) } ?? "")")
    }
    return url
}

private func night(calibrated: Bool = true, measured: Bool = true, top: String = "powernap", drop: Int = 350) -> Night {
    Night(
        id: start, end: end, startPct: 87, endPct: 78, dropPct: 9, dropMah: drop, measured: measured, topCause: top,
        calibrated: calibrated, headline: "You lost 9% (350 mAh) overnight. Mostly Power Nap / maintenance wakes.",
        timeline: [
            TimelineItem(ts: start, text: "Lid closed at 87%; asleep 7.6 of 8.0 h."),
            TimelineItem(ts: nil, text: "Power Nap / maintenance wakes: 31x over 23 min, 43 mAh (est.)."),
            TimelineItem(ts: nil, text: "Fix: sudo pmset -b powernap 0 (undo: sudo pmset -b powernap 1)."),
        ])
}

private func cause(_ key: String, _ mah: Int, count: Int = 0, seconds: Int = 0, estimated: Bool = true, label: String? = nil) -> Cause {
    Cause(key: key, label: label ?? key, mah: mah, seconds: seconds, count: count, estimated: estimated,
          fix: key == "powernap" ? "sudo pmset -b powernap 0" : "", undo: key == "powernap" ? "sudo pmset -b powernap 1" : "")
}

@Test func `store reads drainlogd's rows`() throws {
    let url = try makeDatabase("""
        INSERT INTO nights VALUES (\(start - 86400), \(end - 86400), 92, 90, 2, 80, 3913, 1, 80, 0, 0, 0, 'normal', 1, 0,
          'You lost 2% (80 mAh) overnight. Normal sleep drain.', '[]');
        INSERT INTO nights VALUES (\(start), \(end), 87, 78, 9, 350, 3913, 1, 76, 43, 231, 66.0, 'powernap', 1, 1,
          'You lost 9% (350 mAh) overnight. Mostly Power Nap / maintenance wakes.',
          '[{"text":"Lid closed at 87%.","ts":\(start)},{"text":"Fix: x.","ts":null}]');
        INSERT INTO causes VALUES (\(start), 'baseline', 'baseline sleep drain', 76, 27404, 0, 1, '', '');
        INSERT INTO causes VALUES (\(start), 'powernap', 'Power Nap / maintenance wakes', 43, 1395, 31, 1, 'f', 'u');
        INSERT INTO causes VALUES (\(start), 'unexplained', 'unexplained', 231, 0, 0, 0, '', '');
        INSERT INTO intervals VALUES (\(start), \(start + 60), 'asleep', 'Clamshell Sleep', '', \(start), 'baseline', 1.0, 0);
        INSERT INTO intervals VALUES (\(end), \(end + 99), 'awake', 'x lid y', '', NULL, '', NULL, 0);
        INSERT INTO intervals VALUES (\(end + 99), \(end + 200), 'asleep', 'Idle Sleep', '', NULL, '', NULL, 0);
        INSERT INTO samples VALUES (\(start - 700), 3500, 3913, 89, 0, 'timer');
        INSERT INTO samples VALUES (\(start + 1), 3400, 3913, 87, 0, 'sleep');
        INSERT INTO samples VALUES (\(start + 5), 3400, 3913, 87, 1, 'timer');
        INSERT INTO samples VALUES (\(end + 2), 3050, 3913, 78, 0, 'wake');
        INSERT INTO baseline VALUES (1, 10.0, 'clean', 2, 1, 0.03, 'default', 0, 0);
        """)
    let store = Store(url: url)
    #expect(store.exists)
    let nights = try store.read { try store.nights() }
    #expect(nights.map(\.id) == [start, start - 86400])
    let n = try #require(nights.first)
    #expect(n.measured && n.calibrated && n.topCause == "powernap" && n.dropMah == 350)
    #expect(n.timeline == [TimelineItem(ts: start, text: "Lid closed at 87%."), TimelineItem(ts: nil, text: "Fix: x.")])
    let detail = try store.read { try store.detail(n) }
    #expect(detail.causes.map(\.key) == ["baseline", "powernap", "unexplained"])
    #expect(detail.causes[1].count == 31 && detail.causes[1].estimated && detail.causes[1].fix == "f")
    // The night's intervals and the one that woke the Mac, nothing after.
    #expect(detail.intervals.map(\.start) == [start, end])
    #expect(detail.intervals.map(\.night) == [start, nil])
    // Within 10 min of the night, on battery only.
    #expect(detail.samples == [Sample(ts: start + 1, mah: 3400), Sample(ts: end + 2, mah: 3050)])
    #expect(try store.read { try store.baseline() } == Baseline(mahPerHour: 10, source: "clean", cleanNights: 2, calibrated: true))
    #expect(try store.read { try store.recordingSince() } == Date(timeIntervalSince1970: TimeInterval(start - 700)))
}

@Test func `a missing database is not an error, just empty`() {
    #expect(!Store(url: URL.temporaryDirectory.appending(path: "no-such-\(UUID().uuidString).db")).exists)
}

@Test func `ledger groups DarkWakes by reason and keeps holders apart`() {
    let causes = [
        cause("baseline", 105, seconds: 26040),
        cause("powernap", 255, count: 41, seconds: 2100, label: "Power Nap / maintenance wakes"),
        cause("bluetooth", 60, count: 12, seconds: 500, label: "Bluetooth / Wi-Fi chip wakes"),
        cause("awake", 38, count: 1, seconds: 240, estimated: false, label: "awake time with the lid closed"),
        cause("assertion:caffeinate", 20, seconds: 600, estimated: false, label: "caffeinate holding it awake"),
        cause("unexplained", 32, estimated: false),
    ]
    let rows = Ledger.parts(night(drop: 510), causes)
    #expect(rows.map(\.name) == [
        "Baseline sleep", "DarkWakes", "Power Nap / maintenance", "Bluetooth / Wi-Fi chip", "Awake",
        "Caffeinate holding it awake",
    ])
    #expect(rows.map(\.kind) == [.plain, .group, .child, .child, .plain, .plain])
    #expect(rows[1].mah == 315 && rows[1].approximate && rows[1].detail == "53 wakes, 43 min, by wake reason")
    #expect(rows[4].tag == .measured && rows[4].detail == "4 min in 1 wake")
    #expect(Ledger.unexplained(night(), causes).mah == 32)
    #expect(Ledger.total(night()).tag == .measured)
    #expect(Ledger.total(night(measured: false)).tag == .estimated)
}

@Test func `one wake reason is a plain row, and before calibration the baseline is not measured`() {
    let causes = [cause("baseline", 32, seconds: 27404), cause("powernap", 43, count: 31, seconds: 1395, label: "Power Nap / maintenance wakes"), cause("unexplained", 275, estimated: false)]
    let rows = Ledger.parts(night(calibrated: false), causes)
    #expect(rows.map(\.name) == ["Baseline sleep", "Power Nap / maintenance wakes"])
    #expect(rows[0].tag == .notMeasured && rows[0].detail == "Baseline not measured yet")
    #expect(rows[1].detail == "31 wakes, 23 min")
    #expect(Ledger.unexplained(night(calibrated: false), causes).detail.contains("until the baseline is measured"))
}

@Test func `the fix line shows the top cause's fix`() {
    let causes = [cause("baseline", 76), cause("powernap", 43, count: 31), cause("unexplained", 231)]
    let lines = Incident.list(night(), causes)
    #expect(lines.map(\.time) == [Fmt.clock(start), "All night", "All night"])
    #expect(lines[2].fix?.fix == "sudo pmset -b powernap 0")
    #expect(lines[0].fix == nil && lines[1].fix == nil)
}

@Test func `verdict and sentence`() {
    #expect(night().verdict == "Mostly Power Nap / maintenance wakes.")
    let intervals = [
        Interval(start: start, end: start + 1, state: .darkwake, reason: "Clamshell Sleep", night: start),
        Interval(start: end, end: end + 60, state: .awake, reason: "smc.sysState.Wake lid SMC/UserActivity", night: nil),
    ]
    #expect(night().sentence(intervals) == "Lid closed at \(Fmt.clock(start)) on 87%. Opened at \(Fmt.clock(end)) on 78%.")
    #expect(night().sentence([]) == "Went to sleep at \(Fmt.clock(start)) on 87%. Woke at \(Fmt.clock(end)) on 78%.")
}

@Test func `totals count only the DarkWakes the Mac woke into`() {
    let t = NightTotals(night(), [
        Interval(start: start, end: start + 1, state: .darkwake, reason: "Clamshell Sleep", night: start),
        Interval(start: start + 1, end: start + 900, state: .asleep, reason: "Clamshell Sleep", night: start),
        Interval(start: start + 900, end: start + 945, state: .darkwake, reason: "rtc/Maintenance", night: start),
        Interval(start: start + 945, end: end, state: .asleep, reason: "Maintenance Sleep", night: start),
        Interval(start: end, end: end + 60, state: .awake, reason: "lid", night: nil),
    ])
    #expect(t.wakes == 1 && t.darkWake == 46 && t.awake == 0 && t.asleep == Int(end - start) - 46)
}

@Test(arguments: [(45, "45 s"), (90, "2 min"), (3120, "52 min"), (26040, "7 h 14 min"), (7200, "2 h")])
func `durations read like the mockup`(seconds: Int, text: String) {
    #expect(Fmt.duration(seconds) == text)
}

@Test func `axis ticks every two hours, clear of the ends`() {
    // 23:10 to 07:20 local, as in the mockup.
    var c = DateComponents(year: 2026, month: 11, day: 12, hour: 23, minute: 10)
    let a = Int64(Calendar.current.date(from: c)!.timeIntervalSince1970)
    c.day = 13; c.hour = 7; c.minute = 20
    let b = Int64(Calendar.current.date(from: c)!.timeIntervalSince1970)
    let n = Night(id: a, end: b, startPct: 82, endPct: 68, dropPct: 14, dropMah: 520, measured: true, topCause: "normal",
                  calibrated: true, headline: "", timeline: [])
    let scale = ChartScale(night: n, samples: [Sample(ts: a, mah: 3400), Sample(ts: b, mah: 2880)], lane: 0.09)
    let hours = scale.hourTicks.map { Calendar.current.dateComponents([.hour, .minute], from: $0) }
    #expect(hours.map { [$0.hour!, $0.minute!] } == [[23, 10], [1, 0], [3, 0], [5, 0], [7, 20]])
    #expect(!scale.grid.isEmpty && scale.grid.count <= 3)
    #expect(scale.grid.allSatisfy { scale.y.contains(Double($0)) && $0 % 100 == 0 })
    #expect(scale.laneTop < 2880)
}
