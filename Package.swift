// swift-tools-version: 5.8

import PackageDescription

let package = Package(
	name: "AssetsParser",
	platforms: [
		.iOS(.v15),
	],
	products: [
		.library(name: "AssetsParser", targets: ["AssetsParser"]),
	],
	targets: [
		.binaryTarget(
			name: "CarParserFFI",
			path: "Frameworks/CarParserFFI.xcframework"
		),
		.target(
			name: "AssetsParser",
			dependencies: ["CarParserFFI"],
			path: "Sources/AssetsParser",
			linkerSettings: [
				.linkedLibrary("compression"),
			]
		),
		.testTarget(
			name: "AssetsParserTests",
			dependencies: ["AssetsParser"],
			path: "Tests/AssetsParserTests"
		),
	]
)
