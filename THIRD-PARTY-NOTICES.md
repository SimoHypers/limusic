# Third-party notices: karaoke

Limusic itself is GPL-3.0 ([LICENSE](LICENSE)). The karaoke feature (vocal removal) uses the
components below. None of them is part of the app’s executable or installer: all three runtime
components and the models are downloaded only when the user agrees, on every platform. Their
full licence and notice texts are stored with them: on Windows in `<app data dir>\runtime\onnxruntime-1.24.4\licenses\`,
on Linux/macOS next to the library in `<app data dir>/runtime/onnxruntime-1.24.4/`.

## UVR MDX-Net models (vocal removal)

- Three models from the same source, selectable in Settings → Karaoke: **UVR-MDX-NET-Inst_HQ_3**
  (Strong), **UVR_MDXNET_KARA_2** (Medium) and **UVR_MDXNET_KARA** (Light).
- [Ultimate Vocal Remover](https://github.com/Anjok07/ultimatevocalremovergui) by Anjok07 and
  aufr33. UVR is MIT-licensed; its README asks developers who use its models to "honor the MIT
  license by providing credit to UVR and its developers", which this notice, the download dialog
  and Settings → Karaoke do.
- Each model is downloaded from its official source, the `all_public_uvr_models` release of
  [TRvlvr/model_repo](https://github.com/TRvlvr/model_repo), only when the user asks for it, and
  checked against a pinned size and SHA-256 (`crates/karaoke/src/models.rs`).

## ONNX Runtime 1.24.4

- © Microsoft Corporation, MIT License. On Windows, `onnxruntime.dll` from the NuGet package
  `Microsoft.ML.OnnxRuntime.DirectML` 1.24.4, downloaded from the official NuGet feed (pinned size and SHA-256),
  unmodified. On Linux and macOS, downloaded on first use from the official Microsoft release archive
  (pinned size and SHA-256), extracting the unmodified shared library alongside its LICENSE, ThirdPartyNotices.txt,
  and Privacy.md. The [`ort`](https://github.com/pykeio/ort) crate (MIT OR Apache-2.0) is used only as
  the Rust binding.
- Licence and third-party notices: on Windows, `licenses\onnxruntime-LICENSE.txt` and
  `licenses\onnxruntime-ThirdPartyNotices.txt`. On Linux and macOS, stored as LICENSE and ThirdPartyNotices.txt
  in `<app data dir>/runtime/onnxruntime-1.24.4/`.

## Microsoft DirectML 1.15.4 (Windows)

- `DirectML.dll` from the NuGet package `Microsoft.AI.DirectML` 1.15.4, unmodified (pinned SHA-256),
  downloaded on the user's computer from the official NuGet feed at the user's request (only that one
  file is fetched from the package, using HTTP Range requests), under the Microsoft Software License
  Terms for DirectML (`licenses\DirectML-LICENSE.txt`, `licenses\DirectML-LICENSE-CODE.txt`,
  `licenses\DirectML-ThirdPartyNotices.txt`).
- Those terms state: "The software may collect information about you and your use of the software,
  and send that to Microsoft." DirectML is only loaded when karaoke runs on a graphics card; the
  download dialog says so.

## Microsoft Visual C++ Redistributable (Windows)

- Limusic does not ship or download the VC++ runtime; `onnxruntime.dll` needs the system's Microsoft
  Visual C++ Redistributable 14.44 or newer (vcruntime140.dll, vcruntime140_1.dll, msvcp140.dll,
  msvcp140_1.dll in System32), and the app shows a message with Microsoft's download link when it is
  missing or older.
