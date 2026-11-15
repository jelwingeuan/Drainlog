import AppKit
import Observation
import ServiceManagement
import SwiftUI
import UserNotifications

enum Section: Hashable {
    case lastNight, nights, patterns
}

/// The background recorder, `drainlogd`, as SMAppService sees it.
enum AgentState: Equatable {
    case recording
    case needsApproval
    case notRunning(String)
}

/// App state: the database's rows, refreshed every minute, and where the window is.
@Observable
final class AppModel {
    static let agentPlist = "com.drainlog.agent.plist"

    let store = Store()
    /// Newest first.
    private(set) var nights: [Night] = []
    private(set) var baseline: Baseline?
    private(set) var recordingSince: Date?
    private(set) var error: String?
    private(set) var agent = AppModel.agentState()
    /// Bumped each time `open(night:)` wants the window; the menu bar label opens it.
    private(set) var windowRequests = 0

    var section: Section? = .lastNight
    /// The Nights section's navigation stack: night ids.
    var nightsPath: [Int64] = []

    /// Night details, dropped whenever the rows change. Not observed: filled while views read.
    @ObservationIgnored private var details: [Int64: NightDetail] = [:]

    var latest: Night? { nights.first }

    func night(_ id: Int64) -> Night? { nights.first { $0.id == id } }

    /// Causes, intervals and samples of one night. Read on first use (a few small queries).
    func detail(_ night: Night) -> NightDetail? {
        if let hit = details[night.id] { return hit }
        let loaded = try? store.read { try store.detail(night) }
        details[night.id] = loaded
        return loaded
    }

    // MARK: Navigation

    /// The night the window shows, if it shows one.
    var shownNight: Night? {
        switch section {
        case .lastNight: latest
        case .nights: nightsPath.last.flatMap(night)
        default: nil
        }
    }

    /// The night before (`-1`) or after (`+1`) the shown one.
    func neighbor(_ step: Int) -> Night? {
        guard let shown = shownNight, let i = nights.firstIndex(where: { $0.id == shown.id }) else { return nil }
        return nights.indices.contains(i - step) ? nights[i - step] : nil
    }

    func step(_ step: Int) {
        guard let night = neighbor(step) else { return }
        withAnimation(Motion.spring(reduce: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)) {
            show(night.id)
        }
    }

    /// The latest night is Last Night; any other opens under Nights.
    func show(_ id: Int64) {
        if id == latest?.id {
            section = .lastNight
            nightsPath = []
        } else {
            section = .nights
            nightsPath = [id]
        }
    }

    /// Brings up the window on `night`, or on Last Night.
    func open(night: Int64?) {
        if let night, self.night(night) != nil {
            show(night)
        } else {
            section = .lastNight
            nightsPath = []
        }
        windowRequests += 1
    }

    // MARK: Refresh

    /// Runs for the app's lifetime.
    func start() {
        registerAgent()
        if Bundle.main.isApp {
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert]) { _, _ in }
        }
        Task {
            while true {
                refresh()
                try? await Task.sleep(for: .seconds(60))
            }
        }
    }

    /// The agent rebuilds every 5 min while the Mac is awake; once a minute is plenty.
    func refresh() {
        updateAgent()
        guard store.exists else { return }
        do {
            let (nights, baseline, since) = try store.read {
                (try store.nights(), try store.baseline(), try store.recordingSince())
            }
            if error != nil { error = nil }
            if recordingSince != since { recordingSince = since }
            if nights != self.nights || baseline != self.baseline {
                details = [:]
                self.nights = nights
                self.baseline = baseline
            }
            notifyIfNew()
        } catch {
            self.error = String(describing: error)
        }
    }

    // MARK: Agent

    @ObservationIgnored private var registrationError: String?

    /// Registers drainlogd on first launch (before that, its status reads not registered or not
    /// found). Once registered it is left alone, so turning it off in Login Items sticks.
    private func registerAgent() {
        guard Bundle.main.isApp else { return }
        let service = SMAppService.agent(plistName: Self.agentPlist)
        guard [.notRegistered, .notFound].contains(service.status) else { return }
        do {
            try service.register()
        } catch {
            registrationError = error.localizedDescription
        }
    }

    private func updateAgent() {
        var state = Self.agentState()
        if case .notRunning = state, let registrationError {
            state = .notRunning("Not recording: \(registrationError)")
        }
        if agent != state { agent = state }
    }

    private static func agentState() -> AgentState {
        guard Bundle.main.isApp else { return .notRunning("Not recording: open Drainlog from Drainlog.app.") }
        return switch SMAppService.agent(plistName: agentPlist).status {
        case .enabled: .recording
        case .requiresApproval: .needsApproval
        case .notFound: .notRunning("Not recording: the recorder is missing from Drainlog.app.")
        default: .notRunning("Not recording: the recorder isn't registered.")
        }
    }

    // MARK: Notifications

    private static let notifiedKey = "lastNotifiedNight"

    /// One quiet notification per new night. The first run only takes note of what is there.
    private func notifyIfNew() {
        guard Bundle.main.isApp else { return }
        let defaults = UserDefaults.standard
        let last = defaults.object(forKey: Self.notifiedKey) == nil
            ? nil : Int64(defaults.integer(forKey: Self.notifiedKey))
        let newest = latest?.id ?? 0
        guard newest != last else { return }
        defaults.set(newest, forKey: Self.notifiedKey)
        guard let last, let night = latest, night.id > last else { return }
        let content = UNMutableNotificationContent()
        content.title = "Last night"
        content.body = night.headline
        content.userInfo = ["night": night.id]
        UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: "night-\(night.id)", content: content, trigger: nil))
    }
}

extension Bundle {
    /// SMAppService and notifications need a real app bundle, not `swift run`.
    var isApp: Bool { bundleURL.pathExtension == "app" }
}

enum Motion {
    /// The mockups' critically damped spring, for disclosure and selection only. None under
    /// Reduce Motion.
    static func spring(reduce: Bool) -> Animation? {
        reduce ? nil : .spring(response: 0.35, dampingFraction: 1)
    }
}
