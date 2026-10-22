# Third-party notices: karaoke

Limusic itself is GPL-3.0 ([LICENSE](LICENSE)). The karaoke feature (vocal removal) uses the
components below. None of them is part of the app’s executable: the models are downloaded, 
each only when the user agrees to it, and on Windows the runtime files ship in a separate karaoke-runtime\ 
folder next to the exe and are loaded only when karaoke is turned on. Their full licence and notice texts
are installed with them in `karaoke-runtime\licenses\` (fetched from the official packages by
`scripts/fetch-karaoke-runtime.ps1`).

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

## ONNX Runtime 1.24.4 (DirectML build)

- © Microsoft Corporation, MIT License. On Windows, `onnxruntime.dll` from the NuGet package
  `Microsoft.ML.OnnxRuntime.DirectML` 1.24.4, unmodified (pinned SHA-256). On Linux and macOS, the
  prebuilt static library the [`ort`](https://github.com/pykeio/ort) crate (MIT OR Apache-2.0)
  links in at build time.
- Licence and third-party notices: `karaoke-runtime\licenses\onnxruntime-LICENSE.txt` and
  `onnxruntime-ThirdPartyNotices.txt`.

## Microsoft DirectML 1.15.4 (Windows)

- `DirectML.dll` from the NuGet package `Microsoft.AI.DirectML` 1.15.4, unmodified (pinned
  SHA-256), distributed as part of this application under the Microsoft Software License Terms
  for DirectML (`karaoke-runtime\licenses\DirectML-LICENSE.txt`, with `DirectML-LICENSE-CODE.txt`
  and `DirectML-ThirdPartyNotices.txt`).
- Those terms state: "The software may collect information about you and your use of the software,
  and send that to Microsoft." DirectML is only loaded when karaoke runs on a graphics card; the
  download dialog says so.

## Microsoft Visual C++ runtime (Windows)

- `vcruntime140.dll`, `vcruntime140_1.dll`, `msvcp140.dll` and `msvcp140_1.dll`, which
  `onnxruntime.dll` needs, are deployed app-locally in `karaoke-runtime\` as Distributable Code
  under the Visual Studio license terms. They are taken from the build machine's Visual Studio
  redistributable folder, Microsoft-signed, version 14.44 or later. They are only loaded by
  `onnxruntime.dll`, never by the app itself.
