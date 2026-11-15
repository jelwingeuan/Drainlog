import SwiftUI

/// The morning report for one night: the number and the sentence, the night as a chart, what
/// happened, and the ledger.
struct ReportView: View {
    let night: Night
    @Environment(AppModel.self) private var model
    @ScaledMetric(relativeTo: .body) private var maxWidth: CGFloat = 600

    var body: some View {
        let isLatest = night.id == model.latest?.id
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                if let detail = model.detail(night) {
                    Hero(night: night, sentence: night.sentence(detail.intervals))
                    if !night.calibrated { CalibrationNotice(baseline: model.baseline) }
                    TheNight(night: night, detail: detail)
                    WhatHappened(night: night, causes: detail.causes)
                    LedgerView(night: night, causes: detail.causes).id(night.id)
                } else {
                    Hero(night: night, sentence: nil)
                    Text("Couldn't read this night's details.").foregroundStyle(.supporting)
                }
            }
            .frame(maxWidth: maxWidth, alignment: .leading)
            .padding(.horizontal, 24)
            .padding(.top, 8)
            .padding(.bottom, 28)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(isLatest ? "Last Night" : Fmt.range(night))
        .navigationSubtitle(isLatest ? Fmt.range(night) : "")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                ControlGroup {
                    Button("Previous Night", systemImage: "chevron.left") { model.step(-1) }
                        .help("Previous Night (⌘[)")
                        .disabled(model.neighbor(-1) == nil)
                    Button("Next Night", systemImage: "chevron.right") { model.step(1) }
                        .help("Next Night (⌘])")
                        .disabled(model.neighbor(1) == nil)
                }
            }
        }
    }
}

private struct Hero: View {
    let night: Night
    let sentence: String?
    @ScaledMetric(relativeTo: .largeTitle) private var size: CGFloat = 44

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text("\(night.dropPct)%")
                    .font(.system(size: size, weight: .semibold, design: .rounded))
                    .monospacedDigit()
                HStack(spacing: 7) {
                    Text(Fmt.mah(night.dropMah, approximate: !night.measured))
                        .font(.title3)
                        .monospacedDigit()
                        .foregroundStyle(.supporting)
                    Badge(tag: night.measured ? .measured : .estimated)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(
                "Lost \(night.dropPct) percent, \(Fmt.spokenMah(night.dropMah, approximate: !night.measured)), "
                    + (night.measured ? "measured" : "estimated from Charge percent"))
            Text(night.headline)
                .font(.title.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
            if let sentence {
                Text(sentence).foregroundStyle(.supporting)
            }
        }
        .textSelection(.enabled)
    }
}

private struct CalibrationNotice: View {
    let baseline: Baseline?

    var body: some View {
        let seen = baseline.map { $0.cleanNights < 2 ? " (\($0.cleanNights) of 2 so far)" : "" } ?? ""
        HStack(alignment: .firstTextBaseline, spacing: 9) {
            Image(systemName: "info.circle")
                .foregroundStyle(.tint)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("Baseline not measured yet").fontWeight(.semibold)
                Text(
                    "After two nights with the lid closed and nothing waking the Mac, Drainlog can tell "
                        + "normal sleep drain apart\(seen). Until then the baseline below is a rough "
                        + "estimate, and unexplained may include normal sleep drain."
                )
                .foregroundStyle(.supporting)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.accentColor.opacity(0.1), in: .rect(cornerRadius: 10))
        .accessibilityElement(children: .combine)
    }
}

private struct TheNight: View {
    let night: Night
    let detail: NightDetail

    var body: some View {
        let totals = NightTotals(night, detail.intervals)
        VStack(alignment: .leading, spacing: 9) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("The night").font(.headline)
                Text("\(Fmt.duration(Int(night.end - night.id))) on battery")
                    .foregroundStyle(.supporting)
            }
            NightChart(night: night, detail: detail, style: .full)
            Legend(totals: totals, battery: detail.samples.count > 1)
        }
    }
}

private struct Legend: View {
    let totals: NightTotals
    let battery: Bool

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 18) { items }
            VStack(alignment: .leading, spacing: 5) { items }
        }
        .font(.callout)
        .foregroundStyle(.supporting)
    }

    @ViewBuilder private var items: some View {
        if totals.asleep > 0 {
            item(.asleep) { Text("Asleep \(bold(Fmt.duration(totals.asleep)))") }
        }
        if totals.darkWake > 0 {
            item(.darkWake) {
                totals.wakes > 0
                    ? Text("DarkWake \(bold(Fmt.duration(totals.darkWake))) in \(Fmt.plural(totals.wakes, "wake"))")
                    : Text("DarkWake \(bold(Fmt.duration(totals.darkWake)))")
            }
        }
        if totals.awake > 0 {
            item(.awake) { Text("Awake \(bold(Fmt.duration(totals.awake)))") }
        }
        if battery {
            item(.battery) { Text("Battery") }
        }
    }

    private func bold(_ s: String) -> Text {
        Text(s).fontWeight(.medium).foregroundStyle(.primary)
    }

    private func item(_ swatch: Swatch.Kind, _ text: () -> Text) -> some View {
        HStack(spacing: 6) {
            Swatch(kind: swatch)
            text().monospacedDigit()
        }
        .accessibilityElement(children: .combine)
    }
}

/// The legend's key for one band of the chart.
struct Swatch: View {
    enum Kind { case asleep, darkWake, awake, battery }
    let kind: Kind
    @ScaledMetric(relativeTo: .callout) private var size: CGFloat = 11

    var body: some View {
        Group {
            switch kind {
            case .asleep:
                RoundedRectangle(cornerRadius: 3)
                    .fill(NightChart.asleep)
                    .overlay(RoundedRectangle(cornerRadius: 3).strokeBorder(Color.indigo.opacity(0.45)))
            case .darkWake:
                // Striped like the chart's tick lane: shape, not only hue.
                HStack(spacing: size / 5) {
                    ForEach(0..<3, id: \.self) { _ in Rectangle().fill(Color.orange).frame(width: size / 7) }
                }
                .frame(width: size, height: size)
                .background(NightChart.darkWake)
                .clipShape(.rect(cornerRadius: 3))
            case .awake:
                RoundedRectangle(cornerRadius: 3).fill(NightChart.awake)
            case .battery:
                Capsule().fill(.primary).frame(height: 2)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

private struct WhatHappened: View {
    let night: Night
    let causes: [Cause]
    @ScaledMetric(relativeTo: .body) private var timeWidth: CGFloat = 64
    @Environment(\.openURL) private var openURL

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            Text("What happened").font(.headline)
            VStack(alignment: .leading, spacing: 0) {
                Divider()
                ForEach(Incident.list(night, causes)) { incident in
                    row(incident)
                        .padding(.vertical, 9)
                    Divider()
                }
            }
        }
    }

    @ViewBuilder private func row(_ incident: Incident) -> some View {
        if let cause = incident.fix {
            HStack(alignment: .center, spacing: 12) {
                Text("Fix")
                    .fontWeight(.semibold)
                    .foregroundStyle(.tint)
                    .frame(width: timeWidth, alignment: .leading)
                VStack(alignment: .leading, spacing: 2) {
                    Text(Fmt.fix(cause.fix))
                        .fontWeight(.medium)
                        .fontDesign(Fmt.isCommand(cause.fix) ? .monospaced : nil)
                    Text("Undo: \(cause.undo)").font(.callout).foregroundStyle(.supporting)
                }
                .textSelection(.enabled)
                Spacer(minLength: 8)
                if let pane = settingsPane(for: cause.key) {
                    Button(pane.title) { openURL(pane.url) }
                        .buttonStyle(.borderedProminent)
                }
            }
            .accessibilityElement(children: .contain)
        } else {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(incident.time)
                    .monospacedDigit()
                    .foregroundStyle(.supporting)
                    .frame(width: timeWidth, alignment: .leading)
                Text(incident.text)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }
            .accessibilityElement(children: .combine)
        }
    }
}

private struct LedgerView: View {
    let night: Night
    let causes: [Cause]
    @State private var open: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(night: Night, causes: [Cause]) {
        self.night = night
        self.causes = causes
        // Before calibration the baseline row is the point, so the ledger starts open.
        _open = State(initialValue: !night.calibrated)
    }

    var body: some View {
        VStack(spacing: 0) {
            DisclosureGroup(isExpanded: $open) {
                ForEach(Ledger.parts(night, causes)) { LedgerRow(line: $0, drop: night.dropMah) }
            } label: {
                HStack {
                    Text("Where the \(Fmt.mah(night.dropMah, approximate: !night.measured)) went")
                        .fontWeight(.semibold)
                    Spacer()
                    Text(open ? "Hide ledger" : "Show ledger").foregroundStyle(.supporting)
                }
            }
            .disclosureGroupStyle(LedgerDisclosure(animation: Motion.spring(reduce: reduceMotion)))
            // Always visible, open or not.
            LedgerRow(line: Ledger.unexplained(night, causes), drop: night.dropMah)
            LedgerRow(line: Ledger.total(night), drop: night.dropMah)
        }
        .background(.quinary, in: .rect(cornerRadius: 10))
        .clipShape(.rect(cornerRadius: 10))
    }
}

/// A full-width header row with the chevron on the left, like the mockup's ledger.
private struct LedgerDisclosure: DisclosureGroupStyle {
    let animation: Animation?

    func makeBody(configuration: Configuration) -> some View {
        VStack(spacing: 0) {
            Button {
                withAnimation(animation) { configuration.isExpanded.toggle() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.supporting)
                        .rotationEffect(.degrees(configuration.isExpanded ? 90 : 0))
                    configuration.label
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityValue(configuration.isExpanded ? "Expanded" : "Collapsed")
            if configuration.isExpanded {
                configuration.content
                    .transition(.opacity)
            }
        }
    }
}

private struct LedgerRow: View {
    let line: LedgerLine
    let drop: Int
    @ScaledMetric(relativeTo: .body) private var shareWidth: CGFloat = 36
    @ScaledMetric(relativeTo: .subheadline) private var tagWidth: CGFloat = 96

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 1) {
                Text(line.name)
                    .fontWeight(line.kind == .child ? .regular : line.kind == .group ? .medium : .semibold)
                Text(line.detail)
                    .font(.callout)
                    .foregroundStyle(.supporting)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, line.kind == .child ? 16 : 0)
            Spacer(minLength: 8)
            HStack(spacing: 8) {
                Text(Fmt.mah(line.mah, approximate: line.approximate))
                    .fontWeight(line.kind == .total ? .semibold : .regular)
                Text(Fmt.share(line.mah, of: drop))
                    .foregroundStyle(.supporting)
                    .frame(minWidth: shareWidth, alignment: .trailing)
            }
            .monospacedDigit()
            Badge(tag: line.tag)
                .frame(width: tagWidth, alignment: .trailing)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(line.kind == .unexplained ? Color.orange.opacity(0.09) : .clear)
        .overlay(alignment: .top) { Divider() }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            "\(line.name). \(line.detail). \(Fmt.spokenMah(line.mah, approximate: line.approximate)), "
                + "\(Fmt.share(line.mah, of: drop)) of the drop. \(Badge.title(line.tag)).")
    }
}

/// Measured, Estimated, Unexplained or Not measured: how a number was arrived at.
struct Badge: View {
    let tag: LedgerLine.Tag
    @ScaledMetric(relativeTo: .subheadline) private var dot: CGFloat = 6

    static func title(_ tag: LedgerLine.Tag) -> String {
        switch tag {
        case .measured: "Measured"
        case .estimated: "Estimated"
        case .unexplained: "Unexplained"
        case .notMeasured: "Not measured"
        }
    }

    var body: some View {
        HStack(spacing: 4) {
            switch tag {
            case .measured: Circle().frame(width: dot, height: dot)
            case .estimated: Text("≈").fontWeight(.bold)
            case .unexplained: Text("?").fontWeight(.bold)
            case .notMeasured: Text("–").fontWeight(.bold)
            }
            Text(Self.title(tag))
        }
        .font(.subheadline.weight(.medium))
        .foregroundStyle(.supporting)
        .padding(.horizontal, 7)
        .padding(.vertical, 3)
        .background {
            switch tag {
            case .measured: Capsule().fill(.quaternary)
            case .estimated, .unexplained: Capsule().strokeBorder(.tertiary)
            case .notMeasured: Capsule().strokeBorder(.tertiary, style: StrokeStyle(lineWidth: 1, dash: [3, 2]))
            }
        }
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Self.title(tag))
    }
}

extension ShapeStyle where Self == Color {
    /// Supporting text. The system secondary label colour measures about 3:1 on this window's
    /// light background, under the 4.5:1 text this size needs. The label colour at 80% keeps
    /// its adaptivity (dark mode, Increase Contrast) and clears 4.5:1 in both appearances.
    static var supporting: Color { Color.primary.opacity(0.8) }
}
