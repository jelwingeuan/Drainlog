// swift-tools-version: 6.2
// The Drainlog app: menu bar popover + one window. scripts/bundle.sh wraps it in Drainlog.app.
import PackageDescription

let settings: [SwiftSetting] = [
    .defaultIsolation(MainActor.self),
    .enableUpcomingFeature("NonisolatedNonsendingByDefault"),
    .enableUpcomingFeature("InferIsolatedConformances"),
]

let package = Package(
    name: "Drainlog",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(name: "Drainlog", swiftSettings: settings),
        .testTarget(name: "DrainlogTests", dependencies: ["Drainlog"], swiftSettings: settings),
    ]
)
