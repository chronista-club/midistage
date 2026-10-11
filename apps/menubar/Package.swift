// swift-tools-version: 6.0
// midistage app — メニューバーから始める常駐（docs/spec/01、docs/design/05「常駐と配布」）。
// MenuModel は I/O を持たない純粋な層。SMAppService / endpoint.json / SDK は MidistageApp 側。
import PackageDescription

let package = Package(
    name: "MidistageApp",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "MenuModel", targets: ["MenuModel"]),
        .executable(name: "MidistageApp", targets: ["MidistageApp"]),
    ],
    dependencies: [
        // ルートの Package.swift（MidistageClient）。identity を明示する — path 依存の identity は
        // 既定でディレクトリ名になり、worktree の名前（midistage / midistage-app …）で変わってしまう
        .package(name: "MidistageClient", path: "../.."),
    ],
    targets: [
        .target(name: "MenuModel", path: "Sources/MenuModel"),
        .executableTarget(
            name: "MidistageApp",
            dependencies: ["MenuModel", .product(name: "MidistageClient", package: "MidistageClient")],
            path: "Sources/MidistageApp"),
        .testTarget(name: "MenuModelTests", dependencies: ["MenuModel"], path: "Tests/MenuModelTests"),
    ]
)
