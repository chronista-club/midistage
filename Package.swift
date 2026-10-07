// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "MidistageClient",
    platforms: [.macOS(.v13)],
    products: [.library(name: "MidistageClient", targets: ["MidistageClient"])],
    dependencies: [.package(url: "https://github.com/chronista-club/club-unison.git", from: "2.0.0")],
    targets: [
        .target(name: "MidistageClient", dependencies: [.product(name: "UnisonClient", package: "club-unison")], path: "clients/swift/Sources/MidistageClient"),
        .testTarget(name: "MidistageClientTests", dependencies: ["MidistageClient"], path: "clients/swift/Tests/MidistageClientTests", resources: [.copy("Fixtures")])
    ]
)
