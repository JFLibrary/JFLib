# Text shaping dependencies

JFLib's optional complex text layout uses HarfRust, Unicode Bidi and Unicode
Script, with the transitive dependencies pinned in `Cargo.lock`. These libraries
are statically linked; no HarfBuzz system installation is needed. Cargo and its
registry cache are used during the build. Preserve the license texts when
redistributing binaries. `build_rs.py` copies this notice and `licenses/` into
the target directory's `licenses/text_shape/` folder. The deployed notice preserves the same relative license paths.

The packages below include build-time procedural macros as well as runtime code.
Their code remains under its original licenses, independently of JFLib's own
license. The `licenses` directory contains unmodified license files from the
locked crates.io source packages. Test fonts are local fixtures and are not
redistributed. G'MIC, Paint.NET and third-party plugins are separate installations.

| Package | Upstream license expression | License files |
| --- | --- | --- |
| bitflags-2.13.2 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/bitflags-2.13.2/LICENSE-APACHE), [LICENSE-MIT](licenses/bitflags-2.13.2/LICENSE-MIT) |
| bytemuck-1.25.2 | Zlib OR Apache-2.0 OR MIT | [LICENSE-APACHE](licenses/bytemuck-1.25.2/LICENSE-APACHE), [LICENSE-MIT](licenses/bytemuck-1.25.2/LICENSE-MIT), [LICENSE-ZLIB](licenses/bytemuck-1.25.2/LICENSE-ZLIB) |
| bytemuck_derive-1.12.1 | Zlib OR Apache-2.0 OR MIT | [LICENSE-APACHE](licenses/bytemuck_derive-1.12.1/LICENSE-APACHE), [LICENSE-MIT](licenses/bytemuck_derive-1.12.1/LICENSE-MIT), [LICENSE-ZLIB](licenses/bytemuck_derive-1.12.1/LICENSE-ZLIB) |
| core_maths-0.1.1 | MIT | [LICENSE](licenses/core_maths-0.1.1/LICENSE) |
| font-types-0.12.6 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/font-types-0.12.6/LICENSE-APACHE), [LICENSE-MIT](licenses/font-types-0.12.6/LICENSE-MIT) |
| harfrust-0.14.0 | MIT | [LICENSE](licenses/harfrust-0.14.0/LICENSE) |
| libm-0.2.16 | MIT | [LICENSE.txt](licenses/libm-0.2.16/LICENSE.txt) |
| once_cell-1.21.4 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/once_cell-1.21.4/LICENSE-APACHE), [LICENSE-MIT](licenses/once_cell-1.21.4/LICENSE-MIT) |
| proc-macro2-1.0.107 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/proc-macro2-1.0.107/LICENSE-APACHE), [LICENSE-MIT](licenses/proc-macro2-1.0.107/LICENSE-MIT) |
| quote-1.0.47 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/quote-1.0.47/LICENSE-APACHE), [LICENSE-MIT](licenses/quote-1.0.47/LICENSE-MIT) |
| read-fonts-0.45.0 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/read-fonts-0.45.0/LICENSE-APACHE), [LICENSE-MIT](licenses/read-fonts-0.45.0/LICENSE-MIT) |
| smallvec-1.16.3 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/smallvec-1.16.3/LICENSE-APACHE), [LICENSE-MIT](licenses/smallvec-1.16.3/LICENSE-MIT) |
| spin-0.12.3 | MIT | [LICENSE](licenses/spin-0.12.3/LICENSE) |
| stable_deref_trait-1.2.1 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/stable_deref_trait-1.2.1/LICENSE-APACHE), [LICENSE-MIT](licenses/stable_deref_trait-1.2.1/LICENSE-MIT) |
| syn-3.0.9 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/syn-3.0.9/LICENSE-APACHE), [LICENSE-MIT](licenses/syn-3.0.9/LICENSE-MIT) |
| synstructure-0.14.0 | MIT | [LICENSE](licenses/synstructure-0.14.0/LICENSE) |
| unicode-bidi-0.3.18 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/unicode-bidi-0.3.18/LICENSE-APACHE), [LICENSE-MIT](licenses/unicode-bidi-0.3.18/LICENSE-MIT) |
| unicode-ident-1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | [LICENSE-APACHE](licenses/unicode-ident-1.0.27/LICENSE-APACHE), [LICENSE-MIT](licenses/unicode-ident-1.0.27/LICENSE-MIT), [LICENSE-UNICODE](licenses/unicode-ident-1.0.27/LICENSE-UNICODE) |
| unicode-script-0.5.8 | MIT OR Apache-2.0 | [LICENSE-APACHE](licenses/unicode-script-0.5.8/LICENSE-APACHE), [LICENSE-MIT](licenses/unicode-script-0.5.8/LICENSE-MIT) |
| yoke-0.8.3 | Unicode-3.0 | [LICENSE](licenses/yoke-0.8.3/LICENSE) |
| yoke-derive-0.8.4 | Unicode-3.0 | [LICENSE](licenses/yoke-derive-0.8.4/LICENSE) |
| zerofrom-0.1.8 | Unicode-3.0 | [LICENSE](licenses/zerofrom-0.1.8/LICENSE) |
| zerofrom-derive-0.1.8 | Unicode-3.0 | [LICENSE](licenses/zerofrom-derive-0.1.8/LICENSE) |
