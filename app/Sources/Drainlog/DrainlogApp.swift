import AppKit
import SwiftUI
import UserNotifications

@main
struct DrainlogApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate

    var body: some Scene {
        Window("Drainlog", id: "main") {
            MainWindow()
                .environment(delegate.model)
        }
        .defaultSize(width: 880, height: 820)
        .commands { GoCommands(model: delegate.model) }

        MenuBarExtra {
            Popover()
                .environment(delegate.model)
        } label: {
            MenuBarLabel()
                .environment(delegate.model)
        }
        .menuBarExtraStyle(.window)
    }
}

/// Starts the model, and answers notification clicks by opening that night.
final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    let model = AppModel()

    func applicationDidFinishLaunching(_ notification: Notification) {
        if Bundle.main.isApp { UNUserNotificationCenter.current().delegate = self }
        model.start()
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        // In front, the window already shows the new night: no banner, just Notification Center.
        [.list]
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
    ) async {
        let night = response.notification.request.content.userInfo["night"] as? Int64
        await MainActor.run { model.open(night: night) }
    }
}

/// The menu bar icon. It also opens the window when the model asks (a notification click),
/// since only a view can call `openWindow` and this one is always there.
private struct MenuBarLabel: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Image(systemName: "chart.line.downtrend.xyaxis")
            .accessibilityLabel("Drainlog")
            .onChange(of: model.windowRequests) {
                openWindow(id: "main")
                NSApp.activate()
            }
    }
}

/// Go › Previous Night ⌘[ and Next Night ⌘], the menu twins of the toolbar buttons.
private struct GoCommands: Commands {
    let model: AppModel

    var body: some Commands {
        CommandMenu("Go") {
            Button("Previous Night") { model.step(-1) }
                .keyboardShortcut("[")
                .disabled(model.neighbor(-1) == nil)
            Button("Next Night") { model.step(1) }
                .keyboardShortcut("]")
                .disabled(model.neighbor(1) == nil)
        }
    }
}
