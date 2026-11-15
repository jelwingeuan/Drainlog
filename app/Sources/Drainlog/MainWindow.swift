import ServiceManagement
import SwiftUI

/// The window: Last Night, Nights and Patterns in a sidebar.
struct MainWindow: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            List(selection: $model.section) {
                Label("Last Night", systemImage: "moon.zzz").tag(Section.lastNight)
                Label("Nights", systemImage: "calendar").tag(Section.nights)
                Label("Patterns", systemImage: "chart.bar").tag(Section.patterns)
            }
            .navigationSplitViewColumnWidth(min: 170, ideal: 190)
            .safeAreaInset(edge: .bottom) {
                if model.agent == .recording {
                    AgentStatus(compact: false)
                        .padding(.horizontal, 14)
                        .padding(.bottom, 12)
                }
            }
        } detail: {
            Group {
                switch model.section ?? .lastNight {
                case .lastNight:
                    if let night = model.latest { ReportView(night: night) } else { NoNight(title: "Last Night") }
                case .nights: NightsView()
                case .patterns: PatternsView()
                }
            }
            .frame(minWidth: 520)
            // Not recording is critical, so it sits above the content, not at the sidebar's foot.
            .safeAreaInset(edge: .top, spacing: 0) {
                if model.agent != .recording {
                    AgentStatus(compact: true)
                        .padding(.horizontal, 24)
                        .padding(.vertical, 10)
                        .background(Color.orange.opacity(0.12))
                }
            }
        }
        .environment(\.locale, Fmt.locale)
    }
}

/// The empty state: the agent records, but no night on battery has ended yet.
private struct NoNight: View {
    let title: String
    @Environment(AppModel.self) private var model

    var body: some View {
        ContentUnavailableView {
            Label("No battery night yet", systemImage: "moon.zzz").foregroundStyle(.supporting)
        } description: {
            Text(
                "A night is a lid-close or idle sleep on battery, 2 h or longer, ending when you wake "
                    + "the Mac. The first report comes the morning after."
                    + (model.error.map { "\n\n\($0)" } ?? ""))
            .foregroundStyle(.supporting)
        }
        .navigationTitle(title)
    }
}

private struct NightsView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        NavigationStack(path: $model.nightsPath) {
            Group {
                if model.nights.isEmpty {
                    NoNight(title: "Nights")
                } else {
                    List(model.nights) { night in
                        NavigationLink(value: night.id) { NightRow(night: night) }
                    }
                    .navigationTitle("Nights")
                    .navigationSubtitle(Fmt.plural(model.nights.count, "night") + " on battery")
                }
            }
            .navigationDestination(for: Int64.self) { id in
                if let night = model.night(id) {
                    ReportView(night: night)
                } else {
                    ContentUnavailableView("That night is no longer in the log", systemImage: "calendar")
                }
            }
        }
    }
}

private struct NightRow: View {
    let night: Night

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(Fmt.range(night)).fontWeight(.medium)
                Text(night.verdict)
                    .foregroundStyle(.supporting)
                    .lineLimit(2)
            }
            Spacer()
            Text("\(night.dropPct)%")
                .font(.title3.weight(.semibold))
                .monospacedDigit()
            Text(Fmt.mah(night.dropMah, approximate: !night.measured))
                .monospacedDigit()
                .foregroundStyle(.supporting)
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            "\(Fmt.range(night)). Lost \(night.dropPct) percent, "
                + "\(Fmt.spokenMah(night.dropMah, approximate: !night.measured)). \(night.verdict)")
    }
}

private struct PatternsView: View {
    var body: some View {
        ContentUnavailableView {
            Label("Patterns appear after a few weeks of nights", systemImage: "chart.bar")
                .foregroundStyle(.supporting)
        } description: {
            Text("Each night on battery adds to them.").foregroundStyle(.supporting)
        }
        .navigationTitle("Patterns")
    }
}

/// Whether the background recorder is running, with the way to fix it when it isn't.
struct AgentStatus: View {
    let compact: Bool
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 7) {
            Image(systemName: model.agent == .recording ? "info.circle" : "exclamationmark.triangle")
                .foregroundStyle(model.agent == .recording ? AnyShapeStyle(.supporting) : AnyShapeStyle(.orange))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 6) {
                switch model.agent {
                case .recording:
                    Text(model.recordingSince.map { "Recording since \(Fmt.day($0)). Next report at wake." }
                        ?? "Recording. Next report at wake.")
                case .needsApproval:
                    Text("Not recording yet: allow Drainlog in Login Items.")
                    Button("Open Login Items") { SMAppService.openSystemSettingsLoginItems() }
                        .controlSize(compact ? .regular : .small)
                case .notRunning(let why):
                    Text(why)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
        }
        .font(compact ? .callout : .footnote)
        .foregroundStyle(.supporting)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}
