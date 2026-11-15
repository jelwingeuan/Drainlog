import ServiceManagement
import SwiftUI

/// Last night at a glance from the menu bar: the number, the sentence, a mini timeline, the
/// top cause and one fix.
struct Popover: View {
    @Environment(AppModel.self) private var model
    @ScaledMetric(relativeTo: .body) private var width: CGFloat = 300
    @ScaledMetric(relativeTo: .largeTitle) private var bigSize: CGFloat = 30

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let night = model.latest, let detail = model.detail(night) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text("\(night.dropPct)%")
                            .font(.system(size: bigSize, weight: .semibold, design: .rounded))
                            .monospacedDigit()
                        Text(Fmt.mah(night.dropMah, approximate: !night.measured))
                            .monospacedDigit()
                            .foregroundStyle(.supporting)
                        Badge(tag: night.measured ? .measured : .estimated)
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(
                        "Last night: lost \(night.dropPct) percent, "
                            + "\(Fmt.spokenMah(night.dropMah, approximate: !night.measured))")
                    Text(night.verdict)
                        .font(.title3.weight(.semibold))
                        .fixedSize(horizontal: false, vertical: true)
                }
                NightChart(night: night, detail: detail, style: .compact)
                section { TopCause(night: night, causes: detail.causes) }
            } else {
                VStack(alignment: .leading, spacing: 4) {
                    Text("No battery night yet").font(.headline)
                    Text("A night is a lid-close or idle sleep on battery, 2 h or longer. The first report comes the morning after.")
                        .foregroundStyle(.supporting)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if model.agent != .recording {
                section { AgentStatus(compact: true) }
            }
            section {
                VStack(spacing: 0) {
                    MenuRow(title: "Open Drainlog", shortcut: "⌘O") { model.open(night: nil) }
                        .keyboardShortcut("o")
                    MenuRow(title: "Quit Drainlog", shortcut: "⌘Q") { NSApp.terminate(nil) }
                        .keyboardShortcut("q")
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.top, 13)
        .padding(.bottom, 8)
        .frame(width: width)
        .environment(\.locale, Fmt.locale)
    }

    private func section(@ViewBuilder _ content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Divider()
            content()
        }
    }
}

/// The top cause, then its one fix.
private struct TopCause: View {
    let night: Night
    let causes: [Cause]
    @Environment(\.openURL) private var openURL

    var body: some View {
        let unexplained = causes.first { $0.key == "unexplained" }?.mah ?? 0
        VStack(alignment: .leading, spacing: 10) {
            if let top = night.top(causes) {
                CauseRow(symbol: "sun.max", tint: .orange, title: Fmt.capitalized(top.label)) {
                    HStack(spacing: 6) {
                        Text(
                            (top.count > 0 ? "\(Fmt.plural(top.count, "wake")), " : "\(Fmt.duration(top.seconds)), ")
                                + "\(Fmt.mah(top.mah, approximate: top.estimated)), "
                                + "\(Fmt.share(top.mah, of: night.dropMah)) of the drop")
                        Badge(tag: top.estimated ? .estimated : .measured)
                    }
                }
                if !top.fix.isEmpty {
                    CauseRow(symbol: "wrench.adjustable", tint: .accentColor, title: Fmt.fix(top.fix)) {
                        Text("Undo: \(top.undo)")
                        if let pane = settingsPane(for: top.key) {
                            Button(pane.title) { openURL(pane.url) }
                                .padding(.top, 4)
                        }
                    }
                }
            } else if night.topCause == "cant_tell" {
                CauseRow(symbol: "questionmark", tint: .secondary, title: "Nothing in the logs explains it") {
                    HStack(spacing: 6) {
                        Text("\(Fmt.mah(unexplained)), \(Fmt.share(unexplained, of: night.dropMah)) of the drop")
                        Badge(tag: .unexplained)
                    }
                }
            } else {
                CauseRow(symbol: "moon.zzz", tint: .indigo, title: "Nothing stood out") {
                    Text(night.timeline.dropFirst().first?.text ?? "Normal sleep drain.")
                }
            }
        }
    }
}

private struct CauseRow<Detail: View>: View {
    let symbol: String
    let tint: Color
    let title: String
    @ViewBuilder let detail: Detail
    @ScaledMetric(relativeTo: .body) private var tile: CGFloat = 25

    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            Image(systemName: symbol)
                .font(.callout.weight(.semibold))
                .foregroundStyle(tint)
                .frame(width: tile, height: tile)
                .background(tint.opacity(0.16), in: .rect(cornerRadius: 7))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).fontWeight(.semibold).fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 2) { detail }
                    .font(.callout)
                    .foregroundStyle(.supporting)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .textSelection(.enabled)
        }
        .accessibilityElement(children: .combine)
    }
}

/// A full-width row that highlights like a menu item.
private struct MenuRow: View {
    let title: String
    let shortcut: String
    let action: () -> Void
    @State private var hover = false
    private static let selected = Color(nsColor: .alternateSelectedControlTextColor)

    var body: some View {
        Button(action: action) {
            HStack {
                Text(title)
                Spacer()
                Text(shortcut).foregroundStyle(hover ? AnyShapeStyle(Self.selected) : AnyShapeStyle(.supporting))
                    .accessibilityHidden(true)
            }
            .foregroundStyle(hover ? AnyShapeStyle(Self.selected) : AnyShapeStyle(.primary))
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(hover ? Color.accentColor : .clear, in: .rect(cornerRadius: 6))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, -8)
        .onHover { hover = $0 }
    }
}
