// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "Shop",
    targets: [
        .target(name: "Core"),
        .executableTarget(name: "App", dependencies: ["Core"]),
        .testTarget(name: "CoreTests", dependencies: ["Core"]),
    ]
)
