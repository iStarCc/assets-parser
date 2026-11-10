# assets-parser

Swift package for reading Apple Asset Catalog (`.car`) files, backed by a Rust parser (`rust/`) exposed through a C ABI and shipped as `CarParserFFI.xcframework`.

- Platform: iOS 15+
- Swift API: `Sources/AssetsParser`
- Rust workspace: `rust/` (`bom`, `car`, `deepmap2`, `ffi`)

## Usage

Add the package with SwiftPM and import `AssetsParser`.

## Rebuilding the XCFramework

Requires a Rust toolchain with the `aarch64-apple-ios` and `aarch64-apple-ios-sim` targets. From the host project run `scripts/build-assets-parser.sh`, which builds `rust/ffi` and writes `Frameworks/CarParserFFI.xcframework`.

## License

MIT
