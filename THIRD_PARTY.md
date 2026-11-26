# Third-party notices

Sidenote is AGPL-3.0-or-later. Dependencies retain their own licences.

- [Hex](https://github.com/anomalyco/hex) is the recording and dictation application Sidenote accompanies. Copyright (c) 2026 Christopher Langton, MIT License. Recording capsule source ref: 7ed860ff.
  Sidenote's recording pill adapts Hex's meeting capsule layout and motion.
- [Rune](https://github.com/anomalyco/rune) supplies the UI crates, MIT, pinned to
  a138b97e654b8299b2a4cb518b8476b2b528c2c4 in Cargo.lock.
- Bundled Inter fonts use the SIL Open Font License 1.1. The original notice is
  in [assets/fonts/Inter-LICENSE.txt](assets/fonts/Inter-LICENSE.txt).
- Bundled Fraunces (titles), Figtree (text) and, on Linux, Noto Sans Devanagari and
  Noto Sans Telugu (fallback for Hindi and Telugu) use the same licence. Their notices are
  [Fraunces](assets/fonts/Fraunces-LICENSE.txt), [Figtree](assets/fonts/Figtree-LICENSE.txt),
  [Noto Sans Devanagari](assets/fonts/NotoSansDevanagari-LICENSE.txt) and
  [Noto Sans Telugu](assets/fonts/NotoSansTelugu-LICENSE.txt). The files come from the
  google/fonts repository's `ofl` folder.

The following MIT notice applies to Hex and Rune (upstream notice retained):

```text
MIT License

Copyright (c) 2026 Christopher Langton

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Locked Cargo inventory

Checked against `Cargo.lock` and the cached package manifests, including optional,
build, development and other-platform packages. This lists all 453 dependency
package versions, excluding workspace packages, not just the Windows runtime subset. No incompatible licence
was identified in their declared expressions. For OR expressions, use MIT,
Apache-2.0 or another compatible permissive option; in particular r-efi does not
require its LGPL alternative. Legacy slash expressions denote alternative licences.
Combined AND expressions retain both notices. Unicode-3.0 notices must be retained.

This is a metadata licence screen, not an audit of every dependency's source,
bundled assets, binary redistribution notices or future releases. Preserve upstream
licence and copyright notices when distributing builds. Licence texts can be found
in each downloaded Cargo source package. AGPL requirements apply to this project.
SIL OFL applies separately to the bundled fonts, which are not relicensed as AGPL.

The compatibility screen uses the [GNU licence list](https://www.gnu.org/licenses/license-list.html),
[Apache licensing FAQ](https://www.apache.org/foundation/license-faq.html), and
[Unicode licence terms](https://www.unicode.org/license.txt). The classifications
below are an inference from those terms and the package declarations.

| Package | Version | Declared licence | Screen |
| --- | --- | --- | --- |
| accesskit | 0.25.2 | MIT OR Apache-2.0 | Compatible permissive option |
| accesskit_atspi_common | 0.21.1 | MIT OR Apache-2.0 | Compatible permissive option |
| accesskit_consumer | 0.39.2 | MIT OR Apache-2.0 | Compatible permissive option |
| accesskit_macos | 0.27.2 | MIT OR Apache-2.0 | Compatible permissive option |
| accesskit_unix | 0.24.1 | MIT OR Apache-2.0 | Compatible permissive option |
| accesskit_windows | 0.35.2 | MIT OR Apache-2.0 | Compatible permissive option |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | Compatible permissive option |
| ahash | 0.8.12 | MIT OR Apache-2.0 | Compatible permissive option |
| aho-corasick | 1.1.5 | Unlicense OR MIT | Compatible permissive option |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 | Compatible permissive option |
| android-activity | 0.6.1 | MIT OR Apache-2.0 | Compatible permissive option |
| android-properties | 0.2.2 | MIT | Compatible permissive option |
| android_system_properties | 0.1.6 | MIT OR Apache-2.0 | Compatible permissive option |
| arboard | 3.7.0 | MIT OR Apache-2.0 | Compatible permissive option |
| arrayref | 0.3.9 | BSD-2-Clause | Compatible permissive option |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 | Compatible permissive option |
| as-raw-xcb-connection | 1.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| ash | 0.38.0+1.3.281 | MIT OR Apache-2.0 | Compatible permissive option |
| async-broadcast | 0.7.2 | MIT OR Apache-2.0 | Compatible permissive option |
| async-channel | 2.5.0 | Apache-2.0 OR MIT | Compatible permissive option |
| async-executor | 1.14.1 | Apache-2.0 OR MIT | Compatible permissive option |
| async-io | 2.6.0 | Apache-2.0 OR MIT | Compatible permissive option |
| async-lock | 3.5.0 | Apache-2.0 OR MIT | Compatible permissive option |
| async-process | 2.5.0 | Apache-2.0 OR MIT | Compatible permissive option |
| async-recursion | 1.2.0 | MIT OR Apache-2.0 | Compatible permissive option |
| async-signal | 0.2.14 | Apache-2.0 OR MIT | Compatible permissive option |
| async-task | 4.7.1 | Apache-2.0 OR MIT | Compatible permissive option |
| async-trait | 0.1.92 | MIT OR Apache-2.0 | Compatible permissive option |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | Compatible permissive option |
| atspi | 0.29.0 | Apache-2.0 OR MIT | Compatible permissive option |
| atspi-common | 0.13.0 | Apache-2.0 OR MIT | Compatible permissive option |
| atspi-proxies | 0.13.0 | Apache-2.0 OR MIT | Compatible permissive option |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | Compatible permissive option |
| base64 | 0.23.1 | MIT OR Apache-2.0 | Compatible permissive option |
| bit-set | 0.10.0 | Apache-2.0 OR MIT | Compatible permissive option |
| bit-vec | 0.9.1 | Apache-2.0 OR MIT | Compatible permissive option |
| bitflags | 1.3.2 | MIT/Apache-2.0 | Compatible permissive option |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | Compatible permissive option |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | Compatible permissive option |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 | Compatible permissive option |
| block2 | 0.5.1 | MIT | Compatible permissive option |
| block2 | 0.6.2 | MIT | Compatible permissive option |
| blocking | 1.7.0 | Apache-2.0 OR MIT | Compatible permissive option |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | Compatible permissive option |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| byteorder-lite | 0.1.0 | Unlicense OR MIT | Compatible permissive option |
| bytes | 1.12.1 | MIT | Compatible permissive option |
| calloop | 0.13.0 | MIT | Compatible permissive option |
| calloop-wayland-source | 0.3.0 | MIT | Compatible permissive option |
| cc | 1.11.0 | MIT OR Apache-2.0 | Compatible permissive option |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | Compatible permissive option |
| cfg_aliases | 0.2.2 | MIT | Compatible permissive option |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 | Compatible permissive option |
| chrono | 0.4.45 | MIT OR Apache-2.0 | Compatible permissive option |
| clipboard-win | 5.4.3 | BSL-1.0 | Compatible permissive option |
| codespan-reporting | 0.13.1 | Apache-2.0 | Compatible permissive option |
| combine | 4.6.9 | MIT | Compatible permissive option |
| concurrent-queue | 2.5.0 | Apache-2.0 OR MIT | Compatible permissive option |
| console_error_panic_hook | 0.1.7 | Apache-2.0/MIT | Compatible permissive option |
| const-oid | 0.10.2 | Apache-2.0 OR MIT | Compatible permissive option |
| constant_time_eq | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 | Compatible permissive option |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 | Compatible permissive option |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 | Compatible permissive option |
| core-graphics | 0.23.2 | MIT OR Apache-2.0 | Compatible permissive option |
| core-graphics-types | 0.1.3 | MIT OR Apache-2.0 | Compatible permissive option |
| core_detect | 1.0.0 | MIT/Apache-2.0 | Compatible permissive option |
| cpufeatures | 0.3.1 | MIT OR Apache-2.0 | Compatible permissive option |
| crc32fast | 1.5.4 | MIT OR Apache-2.0 | Compatible permissive option |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 | Compatible permissive option |
| crunchy | 0.2.4 | MIT | Compatible permissive option |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 | Compatible permissive option |
| cursor-icon | 1.2.0 | MIT OR Apache-2.0 OR Zlib | Compatible permissive option |
| digest | 0.11.3 | MIT OR Apache-2.0 | Compatible permissive option |
| dispatch | 0.2.0 | MIT | Compatible permissive option |
| dispatch2 | 0.3.1 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 | Compatible permissive option |
| dlib | 0.5.3 | MIT | Compatible permissive option |
| document-features | 0.2.12 | MIT OR Apache-2.0 | Compatible permissive option |
| downcast-rs | 1.2.1 | MIT/Apache-2.0 | Compatible permissive option |
| dpi | 0.1.2 | Apache-2.0 AND MIT | Compatible permissive option |
| dyn-clone | 1.0.20 | MIT OR Apache-2.0 | Compatible permissive option |
| endi | 1.1.1 | MIT | Compatible permissive option |
| enumflags2 | 0.7.12 | MIT OR Apache-2.0 | Compatible permissive option |
| enumflags2_derive | 0.7.12 | MIT OR Apache-2.0 | Compatible permissive option |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | Compatible permissive option |
| errno | 0.3.14 | MIT OR Apache-2.0 | Compatible permissive option |
| error-code | 3.4.0 | BSL-1.0 | Compatible permissive option |
| event-listener | 5.4.2 | Apache-2.0 OR MIT | Compatible permissive option |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT | Compatible permissive option |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | Compatible permissive option |
| fax | 0.2.7 | MIT | Compatible permissive option |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 | Compatible permissive option |
| find-msvc-tools | 0.1.15 | MIT OR Apache-2.0 | Compatible permissive option |
| fixedbitset | 0.5.7 | MIT OR Apache-2.0 | Compatible permissive option |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | Compatible permissive option |
| foldhash | 0.1.5 | Zlib | Compatible permissive option |
| foldhash | 0.2.0 | Zlib | Compatible permissive option |
| font-types | 0.12.7 | MIT OR Apache-2.0 | Compatible permissive option |
| fontique | 0.12.0 | Apache-2.0 OR MIT | Compatible permissive option |
| foreign-types | 0.5.0 | MIT/Apache-2.0 | Compatible permissive option |
| foreign-types-macros | 0.2.4 | MIT/Apache-2.0 | Compatible permissive option |
| foreign-types-shared | 0.3.1 | MIT/Apache-2.0 | Compatible permissive option |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | Compatible permissive option |
| futures | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| futures-executor | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| futures-lite | 2.6.1 | Apache-2.0 OR MIT | Compatible permissive option |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| gethostname | 1.1.0 | Apache-2.0 | Compatible permissive option |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | Compatible permissive option |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | Compatible permissive option |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | Compatible permissive option |
| gpu-allocator | 0.28.0 | MIT OR Apache-2.0 | Compatible permissive option |
| half | 2.7.1 | MIT OR Apache-2.0 | Compatible permissive option |
| harfrust | 0.12.0 | MIT | Compatible permissive option |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 | Compatible permissive option |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 | Compatible permissive option |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | Compatible permissive option |
| hermit-abi | 0.5.3 | MIT OR Apache-2.0 | Compatible permissive option |
| hex | 0.4.3 | MIT OR Apache-2.0 | Compatible permissive option |
| http | 1.5.0 | MIT OR Apache-2.0 | Compatible permissive option |
| httparse | 1.10.1 | MIT OR Apache-2.0 | Compatible permissive option |
| hybrid-array | 0.4.15 | MIT OR Apache-2.0 | Compatible permissive option |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 | Compatible permissive option |
| iana-time-zone-haiku | 0.1.2 | MIT OR Apache-2.0 | Compatible permissive option |
| icu_collections | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_locale_core | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_locale_fallback | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_locale_fallback_data | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_normalizer | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_normalizer_data | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_properties | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_properties_data | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_provider | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_segmenter | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| icu_segmenter_data | 2.4.0 | Unicode-3.0 | Compatible permissive option |
| idna | 1.1.0 | MIT OR Apache-2.0 | Compatible permissive option |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT | Compatible permissive option |
| image | 0.25.10 | MIT OR Apache-2.0 | Compatible permissive option |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | Compatible permissive option |
| itoa | 1.0.18 | MIT OR Apache-2.0 | Compatible permissive option |
| jni | 0.22.4 | MIT OR Apache-2.0 | Compatible permissive option |
| jni-macros | 0.22.4 | MIT OR Apache-2.0 | Compatible permissive option |
| jni-sys | 0.3.1 | MIT OR Apache-2.0 | Compatible permissive option |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 | Compatible permissive option |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 | Compatible permissive option |
| jobserver | 0.1.35 | MIT OR Apache-2.0 | Compatible permissive option |
| js-sys | 0.3.107 | MIT OR Apache-2.0 | Compatible permissive option |
| libc | 0.2.191 | MIT OR Apache-2.0 | Compatible permissive option |
| libloading | 0.8.9 | ISC | Compatible permissive option |
| libm | 0.2.16 | MIT | Compatible permissive option |
| libredox | 0.1.25 | MIT | Compatible permissive option |
| linebender_resource_handle | 0.1.1 | Apache-2.0 OR MIT | Compatible permissive option |
| linux-raw-sys | 0.4.15 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compatible permissive option |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compatible permissive option |
| litemap | 0.8.3 | Unicode-3.0 | Compatible permissive option |
| litrs | 1.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | Compatible permissive option |
| log | 0.4.35 | MIT OR Apache-2.0 | Compatible permissive option |
| memchr | 2.8.3 | Unlicense OR MIT | Compatible permissive option |
| memmap2 | 0.9.12 | MIT OR Apache-2.0 | Compatible permissive option |
| memoffset | 0.9.1 | MIT | Compatible permissive option |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | Compatible permissive option |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | Compatible permissive option |
| mio | 1.2.4 | MIT | Compatible permissive option |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 | Compatible permissive option |
| naga | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| naga-types | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| ndk | 0.9.0 | MIT OR Apache-2.0 | Compatible permissive option |
| ndk-context | 0.1.1 | MIT OR Apache-2.0 | Compatible permissive option |
| ndk-sys | 0.6.0+11769913 | MIT OR Apache-2.0 | Compatible permissive option |
| nom | 8.0.0 | MIT | Compatible permissive option |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | Compatible permissive option |
| num_enum | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 | Compatible permissive option |
| num_enum_derive | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 | Compatible permissive option |
| objc-sys | 0.3.5 | MIT | Compatible permissive option |
| objc2 | 0.5.3 | MIT | Compatible permissive option |
| objc2 | 0.6.5 | MIT | Compatible permissive option |
| objc2-app-kit | 0.2.2 | MIT | Compatible permissive option |
| objc2-app-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-av-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-cloud-kit | 0.2.2 | MIT | Compatible permissive option |
| objc2-contacts | 0.2.2 | MIT | Compatible permissive option |
| objc2-core-audio | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-core-audio-types | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-core-data | 0.2.2 | MIT | Compatible permissive option |
| objc2-core-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-core-graphics | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-core-image | 0.2.2 | MIT | Compatible permissive option |
| objc2-core-location | 0.2.2 | MIT | Compatible permissive option |
| objc2-core-media | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-core-video | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-encode | 4.1.0 | MIT | Compatible permissive option |
| objc2-foundation | 0.2.2 | MIT | Compatible permissive option |
| objc2-foundation | 0.3.2 | MIT | Compatible permissive option |
| objc2-io-surface | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-link-presentation | 0.2.2 | MIT | Compatible permissive option |
| objc2-metal | 0.2.2 | MIT | Compatible permissive option |
| objc2-metal | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-quartz-core | 0.2.2 | MIT | Compatible permissive option |
| objc2-quartz-core | 0.3.2 | Zlib OR Apache-2.0 OR MIT | Compatible permissive option |
| objc2-symbols | 0.2.2 | MIT | Compatible permissive option |
| objc2-ui-kit | 0.2.2 | MIT | Compatible permissive option |
| objc2-uniform-type-identifiers | 0.2.2 | MIT | Compatible permissive option |
| objc2-user-notifications | 0.2.2 | MIT | Compatible permissive option |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | Compatible permissive option |
| orbclient | 0.3.55 | MIT | Compatible permissive option |
| ordered-float | 5.5.1 | MIT | Compatible permissive option |
| ordered-stream | 0.2.0 | MIT OR Apache-2.0 | Compatible permissive option |
| os_pipe | 1.2.3 | MIT | Compatible permissive option |
| parking | 2.2.1 | Apache-2.0 OR MIT | Compatible permissive option |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | Compatible permissive option |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | Compatible permissive option |
| parlance | 0.1.1 | Apache-2.0 OR MIT | Compatible permissive option |
| parley | 0.12.0 | Apache-2.0 OR MIT | Compatible permissive option |
| parley_data | 0.12.0 | Apache-2.0 OR MIT | Compatible permissive option |
| parley_emoji | 0.12.0 | MIT | Compatible permissive option |
| parley_engine | 0.12.0 | Apache-2.0 OR MIT | Compatible permissive option |
| pastey | 0.2.3 | MIT OR Apache-2.0 | Compatible permissive option |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | Compatible permissive option |
| petgraph | 0.8.3 | MIT OR Apache-2.0 | Compatible permissive option |
| phf | 0.14.0 | MIT | Compatible permissive option |
| phf_generator | 0.14.0 | MIT | Compatible permissive option |
| phf_macros | 0.14.0 | MIT | Compatible permissive option |
| phf_shared | 0.14.0 | MIT | Compatible permissive option |
| pin-project | 1.1.13 | Apache-2.0 OR MIT | Compatible permissive option |
| pin-project-internal | 1.1.13 | Apache-2.0 OR MIT | Compatible permissive option |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | Compatible permissive option |
| piper | 0.2.5 | MIT OR Apache-2.0 | Compatible permissive option |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 | Compatible permissive option |
| plain | 0.2.3 | MIT/Apache-2.0 | Compatible permissive option |
| png | 0.18.1 | MIT OR Apache-2.0 | Compatible permissive option |
| polling | 3.11.0 | Apache-2.0 OR MIT | Compatible permissive option |
| pollster | 1.0.1 | Apache-2.0/MIT | Compatible permissive option |
| portable-atomic | 1.15.0 | Apache-2.0 OR MIT | Compatible permissive option |
| portable-atomic-util | 0.2.8 | Apache-2.0 OR MIT | Compatible permissive option |
| potential_utf | 0.1.6 | Unicode-3.0 | Compatible permissive option |
| presser | 0.3.1 | MIT OR Apache-2.0 | Compatible permissive option |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 | Compatible permissive option |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | Compatible permissive option |
| profiling | 1.0.18 | MIT OR Apache-2.0 | Compatible permissive option |
| proptest | 1.12.0 | MIT OR Apache-2.0 | Compatible permissive option |
| pulldown-cmark | 0.13.6 | MIT | Compatible permissive option |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 | Compatible permissive option |
| quick-error | 2.0.1 | MIT/Apache-2.0 | Compatible permissive option |
| quick-xml | 0.41.0 | MIT | Compatible permissive option |
| quote | 1.0.47 | MIT OR Apache-2.0 | Compatible permissive option |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | Compatible permissive option |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | Compatible permissive option |
| rand | 0.10.3 | MIT OR Apache-2.0 | Compatible permissive option |
| rand_core | 0.10.1 | MIT OR Apache-2.0 | Compatible permissive option |
| rand_xorshift | 0.5.0 | MIT OR Apache-2.0 | Compatible permissive option |
| range-alloc | 0.1.5 | MIT OR Apache-2.0 | Compatible permissive option |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | Compatible permissive option |
| raw-window-metal | 1.1.0 | MIT OR Apache-2.0 | Compatible permissive option |
| read-fonts | 0.41.0 | MIT OR Apache-2.0 | Compatible permissive option |
| redox_syscall | 0.4.1 | MIT | Compatible permissive option |
| redox_syscall | 0.5.18 | MIT | Compatible permissive option |
| redox_syscall | 0.9.5 | MIT | Compatible permissive option |
| ref-cast | 1.0.27 | MIT OR Apache-2.0 | Compatible permissive option |
| ref-cast-impl | 1.0.27 | MIT OR Apache-2.0 | Compatible permissive option |
| regex | 1.13.1 | MIT OR Apache-2.0 | Compatible permissive option |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 | Compatible permissive option |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 | Compatible permissive option |
| renderdoc-sys | 1.1.0 | MIT OR Apache-2.0 | Compatible permissive option |
| ring | 0.17.14 | Apache-2.0 AND ISC | Compatible, retain both notices |
| rmcp | 3.9.0 | Apache-2.0 | Compatible permissive option |
| rune | 0.0.1 | MIT | Compatible permissive option |
| rune-kit | 0.0.1 | MIT | Compatible permissive option |
| rune-motion | 0.1.0 | MIT | Compatible permissive option |
| rune-staging | 0.0.1 | MIT | Compatible permissive option |
| rune-taffy | 0.0.1 | MIT | Compatible permissive option |
| rune-theme-opencode | 0.0.1 | MIT | Compatible permissive option |
| rustc-demangle | 0.1.28 | MIT/Apache-2.0 | Compatible permissive option |
| rustc-hash | 1.1.0 | Apache-2.0/MIT | Compatible permissive option |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT | Compatible permissive option |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 | Compatible permissive option |
| rustix | 0.38.44 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compatible permissive option |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compatible permissive option |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT | Compatible permissive option |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 | Compatible permissive option |
| rustls-webpki | 0.103.16 | ISC | Compatible permissive option |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | Compatible permissive option |
| same-file | 1.0.6 | Unlicense/MIT | Compatible permissive option |
| schemars | 1.2.3 | MIT | Compatible permissive option |
| schemars_derive | 1.2.3 | MIT | Compatible permissive option |
| scoped-tls | 1.0.1 | MIT/Apache-2.0 | Compatible permissive option |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | Compatible permissive option |
| sctk-adwaita | 0.10.1 | MIT | Compatible permissive option |
| semver | 1.0.28 | MIT OR Apache-2.0 | Compatible permissive option |
| serde | 1.0.229 | MIT OR Apache-2.0 | Compatible permissive option |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | Compatible permissive option |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | Compatible permissive option |
| serde_derive_internals | 0.30.0 | MIT OR Apache-2.0 | Compatible permissive option |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | Compatible permissive option |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 | Compatible permissive option |
| sha1_smol | 1.0.1 | BSD-3-Clause | Compatible permissive option |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | Compatible permissive option |
| shlex | 2.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 | Compatible permissive option |
| simd-adler32 | 0.3.10 | MIT | Compatible permissive option |
| simd_cesu8 | 1.2.0 | Apache-2.0 OR MIT | Compatible permissive option |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 | Compatible permissive option |
| siphasher | 1.0.4 | MIT OR Apache-2.0 | Compatible permissive option |
| skrifa | 0.44.0 | MIT OR Apache-2.0 | Compatible permissive option |
| slab | 0.4.12 | MIT | Compatible permissive option |
| slotmap | 1.1.1 | Zlib | Compatible permissive option |
| smallvec | 1.16.4 | MIT OR Apache-2.0 | Compatible permissive option |
| smithay-client-toolkit | 0.19.2 | MIT | Compatible permissive option |
| smol_str | 0.2.2 | MIT OR Apache-2.0 | Compatible permissive option |
| socket2 | 0.6.5 | MIT OR Apache-2.0 | Compatible permissive option |
| spirv | 0.4.0+sdk-1.4.341.0 | Apache-2.0 | Compatible permissive option |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | Compatible permissive option |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 | Compatible permissive option |
| strict-num | 0.1.1 | MIT | Compatible permissive option |
| subtle | 2.6.1 | BSD-3-Clause | Compatible permissive option |
| swash | 0.2.10 | Apache-2.0 OR MIT | Compatible permissive option |
| syn | 2.0.119 | MIT OR Apache-2.0 | Compatible permissive option |
| syn | 3.0.9 | MIT OR Apache-2.0 | Compatible permissive option |
| synstructure | 0.14.0 | MIT | Compatible permissive option |
| taffy | 0.15.1 | MIT | Compatible permissive option |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Compatible permissive option |
| termcolor | 1.4.1 | Unlicense OR MIT | Compatible permissive option |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | Compatible permissive option |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Compatible permissive option |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 | Compatible permissive option |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | Compatible permissive option |
| tiff | 0.11.3 | MIT | Compatible permissive option |
| tiny-skia | 0.11.4 | BSD-3-Clause | Compatible permissive option |
| tiny-skia-path | 0.11.4 | BSD-3-Clause | Compatible permissive option |
| tinystr | 0.8.5 | Unicode-3.0 | Compatible permissive option |
| tokio | 1.54.1 | MIT | Compatible permissive option |
| tokio-macros | 2.7.2 | MIT | Compatible permissive option |
| tokio-util | 0.7.20 | MIT | Compatible permissive option |
| toml_datetime | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 | Compatible permissive option |
| toml_edit | 0.25.18+spec-1.1.0 | MIT OR Apache-2.0 | Compatible permissive option |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 | Compatible permissive option |
| tracing | 0.1.44 | MIT | Compatible permissive option |
| tracing-attributes | 0.1.31 | MIT | Compatible permissive option |
| tracing-core | 0.1.36 | MIT | Compatible permissive option |
| tree_magic_mini | 3.2.2 | MIT | Compatible permissive option |
| typenum | 1.20.1 | MIT OR Apache-2.0 | Compatible permissive option |
| uds_windows | 1.2.1 | MIT | Compatible permissive option |
| unarray | 0.1.4 | MIT OR Apache-2.0 | Compatible permissive option |
| unicase | 2.10.0 | MIT OR Apache-2.0 | Compatible permissive option |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | Compatible permissive option |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 | Compatible permissive option |
| unicode-width | 0.2.3 | MIT OR Apache-2.0 | Compatible permissive option |
| untrusted | 0.9.0 | ISC | Compatible permissive option |
| ureq | 3.5.1 | MIT OR Apache-2.0 | Compatible permissive option |
| ureq-proto | 0.7.1 | MIT OR Apache-2.0 | Compatible permissive option |
| url | 2.5.8 | MIT OR Apache-2.0 | Compatible permissive option |
| utf8-zero | 0.8.1 | MIT OR Apache-2.0 | Compatible permissive option |
| utf8_iter | 1.1.0 | Apache-2.0 OR MIT | Compatible permissive option |
| utf_types | 1.0.0 | Apache-2.0 OR MIT | Compatible permissive option |
| uuid | 1.29.0 | Apache-2.0 OR MIT | Compatible permissive option |
| version_check | 0.9.5 | MIT/Apache-2.0 | Compatible permissive option |
| walkdir | 2.5.0 | Unlicense/MIT | Compatible permissive option |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compatible permissive option |
| wasip2 | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compatible permissive option |
| wasm-bindgen | 0.2.130 | MIT OR Apache-2.0 | Compatible permissive option |
| wasm-bindgen-futures | 0.4.80 | MIT OR Apache-2.0 | Compatible permissive option |
| wasm-bindgen-macro | 0.2.130 | MIT OR Apache-2.0 | Compatible permissive option |
| wasm-bindgen-macro-support | 0.2.130 | MIT OR Apache-2.0 | Compatible permissive option |
| wasm-bindgen-shared | 0.2.130 | MIT OR Apache-2.0 | Compatible permissive option |
| wayland-backend | 0.3.17 | MIT | Compatible permissive option |
| wayland-client | 0.31.15 | MIT | Compatible permissive option |
| wayland-csd-frame | 0.3.0 | MIT | Compatible permissive option |
| wayland-cursor | 0.31.14 | MIT | Compatible permissive option |
| wayland-protocols | 0.32.13 | MIT | Compatible permissive option |
| wayland-protocols-plasma | 0.3.12 | MIT | Compatible permissive option |
| wayland-protocols-wlr | 0.3.12 | MIT | Compatible permissive option |
| wayland-scanner | 0.31.11 | MIT | Compatible permissive option |
| wayland-sys | 0.31.11 | MIT | Compatible permissive option |
| web-sys | 0.3.107 | MIT OR Apache-2.0 | Compatible permissive option |
| web-time | 1.1.0 | MIT OR Apache-2.0 | Compatible permissive option |
| webpki-roots | 1.0.11 | CDLA-Permissive-2.0 | Certificate data, retain licence notice |
| weezl | 0.1.12 | MIT OR Apache-2.0 | Compatible permissive option |
| wgpu | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| wgpu-core | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| wgpu-core-deps-apple | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| wgpu-core-deps-windows-linux-android | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| wgpu-hal | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| wgpu-naga-bridge | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| wgpu-types | 30.0.1 | MIT OR Apache-2.0 | Compatible permissive option |
| winapi-util | 0.1.11 | Unlicense OR MIT | Compatible permissive option |
| windows | 0.62.2 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-future | 0.3.2 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 | Compatible permissive option |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 | Compatible permissive option |
| winit | 0.30.13 | Apache-2.0 | Compatible permissive option |
| winnow | 1.0.5 | MIT | Compatible permissive option |
| wit-bindgen | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compatible permissive option |
| wl-clipboard-rs | 0.9.5 | MIT OR Apache-2.0 | Compatible permissive option |
| writeable | 0.6.4 | Unicode-3.0 | Compatible permissive option |
| x11-dl | 2.21.0 | MIT | Compatible permissive option |
| x11rb | 0.13.2 | MIT OR Apache-2.0 | Compatible permissive option |
| x11rb-protocol | 0.13.2 | MIT OR Apache-2.0 | Compatible permissive option |
| xcursor | 0.3.11 | MIT | Compatible permissive option |
| xkbcommon | 0.9.0 | MIT | Compatible permissive option |
| xkbcommon-dl | 0.4.2 | MIT | Compatible permissive option |
| xkeysym | 0.2.1 | MIT OR Apache-2.0 OR Zlib | Compatible permissive option |
| yazi | 0.2.2 | Apache-2.0 OR MIT | Compatible permissive option |
| yoke | 0.8.4 | Unicode-3.0 | Compatible permissive option |
| yoke-derive | 0.8.4 | Unicode-3.0 | Compatible permissive option |
| zbus | 5.19.0 | MIT | Compatible permissive option |
| zbus-lockstep | 0.5.2 | MIT | Compatible permissive option |
| zbus-lockstep-macros | 0.5.2 | MIT | Compatible permissive option |
| zbus_macros | 5.19.0 | MIT | Compatible permissive option |
| zbus_names | 4.3.5 | MIT | Compatible permissive option |
| zbus_xml | 5.2.1 | MIT | Compatible permissive option |
| zcheapstr | 1.2.0 | MIT | Compatible permissive option |
| zeno | 0.3.3 | Apache-2.0 OR MIT | Compatible permissive option |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | Compatible permissive option |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | Compatible permissive option |
| zerofrom | 0.1.8 | Unicode-3.0 | Compatible permissive option |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | Compatible permissive option |
| zeroize | 1.9.1 | Apache-2.0 OR MIT | Compatible permissive option |
| zerotrie | 0.2.5 | Unicode-3.0 | Compatible permissive option |
| zerovec | 0.11.9 | Unicode-3.0 | Compatible permissive option |
| zerovec-derive | 0.11.7 | Unicode-3.0 | Compatible permissive option |
| zlib-rs | 0.6.8 | Zlib | Compatible permissive option |
| zmij | 1.0.23 | MIT | Compatible permissive option |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib | Compatible permissive option |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib | Compatible permissive option |
| zvariant | 5.15.1 | MIT | Compatible permissive option |
| zvariant_derive | 5.15.1 | MIT | Compatible permissive option |
| zvariant_utils | 4.2.0 | MIT | Compatible permissive option |

## Model download dependencies

- [ureq](https://docs.rs/ureq/), MIT OR Apache-2.0, supplies blocking HTTP on a worker with rustls. It provides streaming bodies, Range headers, explicit redirect control and bounded network reads without an async runtime. Default features are disabled; only rustls is enabled, with no cookies, compression, archive extraction or remote code. Keep the locked version and its Mozilla root bundle current.
- [sha2](https://docs.rs/sha2/), MIT OR Apache-2.0, verifies catalogue SHA-256 before files become installable. [url](https://docs.rs/url/), MIT OR Apache-2.0, parses and resolves redirects before enforcing the HTTPS host allowlist. sha2 was already in the dependency tree; url is added explicitly for redirect validation.
- Models are downloaded only on request. IndicConformer exports retain AI4Bharat's MIT terms; Oriserve Swift and CIIL Anuvani retain Apache-2.0. Source pages are pinned in the catalogue. Nemotron is a Hex-managed reference under OpenMDW-1.1; Sidenote does not redistribute its weights.

The Mozilla certificate data in webpki-roots uses [CDLA-Permissive-2.0](https://cdla.dev/permissive-2-0/). The full text is retained in [assets/licenses/webpki-roots-CDLA-Permissive-2.0.txt](assets/licenses/webpki-roots-CDLA-Permissive-2.0.txt) and must accompany distributed certificate data. It is separate from the software licences above.

Lucide 1.61.0 icons: ISC, with Feather-derived icons under MIT. Both notices are in `assets/licenses/lucide-LICENSE.txt`, included in the app package.

Solar Linear icons by 480 Design and Damir Tuarshe use CC BY 4.0. The shape data
comes from [Iconify’s pinned Solar snapshot](https://github.com/iconify/icon-sets/blob/d1fdcf54272b299075b4fbf21851147e8c3846f9/json/solar.json).
Sidenote normalizes the shape attributes and adds two stroke widths.
See [the Solar notice](assets/licenses/solar-LICENSE.txt) for attribution and licence terms.
