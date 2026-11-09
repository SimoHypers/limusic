# Third-party notices: karaoke

Limusic itself is GPL-3.0 ([LICENSE](LICENSE)). The karaoke feature (vocal removal) uses the
components below. The models, ONNX Runtime and DirectML are not part of the app’s executable or installer:
they are downloaded only when the user agrees, on every platform. The only files of this feature in the
Windows installer are the Microsoft Visual C++ runtime DLLs described at the end. Their
full licence and notice texts are stored with them: on Windows in `<app data dir>\runtime\onnxruntime-1.24.4\licenses\`,
on Linux/macOS next to the library in `<app data dir>/runtime/onnxruntime-1.24.4/`.

## UVR MDX-Net models (vocal removal)

- Three models from the same source, selectable in Settings → Karaoke: **UVR-MDX-NET-Inst_HQ_3**
  (Best), **UVR-MDX-NET-Inst_Main** (Balanced) and **UVR_MDXNET_3_9662** (Light).
- From [Ultimate Vocal Remover](https://github.com/Anjok07/ultimatevocalremovergui) by Anjok07 and
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

## ONNX Runtime WebGPU execution provider (Linux)

- The package `onnxruntime-ep-webgpu` 0.5.0 (the same binary as NuGet `Microsoft.ML.OnnxRuntime.EP.WebGpu` 0.5.0), MIT license, downloaded on the user's computer from files.pythonhosted.org with a pinned size and SHA-256, unmodified.
- Loaded only when karaoke runs on the graphics card.

## Microsoft Visual C++ Redistributable (Windows)

- Limusic ships four Microsoft Visual C++ runtime DLLs in the Windows installer, in the `vcruntime` folder next to the app: vcruntime140.dll, vcruntime140_1.dll, msvcp140.dll and msvcp140_1.dll (under 1 MB in total, version 14.44 or newer), unmodified, copied from the Visual Studio redistributable folder that Microsoft's licence terms allow applications to redistribute. `onnxruntime.dll` needs them. They are only loaded when karaoke is turned on; the loader refuses a copy older than 14.44, and if the folder is missing it uses the system's copy instead (the Microsoft Visual C++ Redistributable 14.44 or newer), showing a message with Microsoft's download link when that is missing or older.
