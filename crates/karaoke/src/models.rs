use crate::mdx::MdxParams;

/// The three quality tiers. Settings and the UI store the tier (`key`), never the model behind it,
/// so a better model can replace one without touching either; the cache is keyed by the model file
/// (`cache_tag`), so the swap never plays an instrumental the old model made.
///
/// All three take out every vocal. The first pass shipped UVR's two karaoke models as Light and
/// Balanced, which remove the lead vocal and keep the backing vocals by design: on a test chorus
/// they left 70% and 66% of the vocals Best removes. The models now in those tiers have the same
/// size and speed and leave 4% and 0.4% (Fedora, i5-8400, 2 threads, 60 s, times real time):
///
/// | tier     | model                  | time  | RAM    |
/// |----------|------------------------|-------|--------|
/// | Light    | UVR_MDXNET_3_9662      | 0.43x | 1.2 GB |
/// | Balanced | UVR-MDX-NET-Inst_Main  | 0.86x | 1.8 GB |
/// | Best     | UVR-MDX-NET-Inst_HQ_3  | 1.37x | 2.7 GB |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelId {
    Light,
    Balanced,
    Best,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryStem {
    Instrumental,
    Vocals,
}

#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub id: ModelId,
    pub key: &'static str,
    pub file_name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// Exactly UVR's `model_data.json` entry for this file (UVR keys it by the md5 of the file's
    /// last 10000 KiB): the network was trained on spectrograms of that shape, and a different
    /// `n_fft` feeds it every frequency in the wrong place.
    pub params: MdxParams,
    pub primary_stem: PrimaryStem,
}

impl ModelId {
    pub const ALL: [ModelId; 3] = [ModelId::Light, ModelId::Balanced, ModelId::Best];
    pub const DEFAULT: ModelId = ModelId::Light;

    pub fn spec(self) -> ModelSpec {
        match self {
            ModelId::Light => ModelSpec {
                id: self,
                key: "light",
                file_name: "UVR_MDXNET_3_9662.onnx",
                url: concat!(
                    "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/",
                    "UVR_MDXNET_3_9662.onnx"
                ),
                size: 29_704_436,
                sha256: "e02220e80d8253f4c2209f8924298b2b686bbdf2868b788ff5500fb9bd94aadc",
                params: MdxParams {
                    n_fft: 6144,
                    hop: 1024,
                    dim_f: 2048,
                    dim_t: 256,
                    compensate: 1.035,
                },
                primary_stem: PrimaryStem::Vocals,
            },
            ModelId::Balanced => ModelSpec {
                id: self,
                key: "balanced",
                file_name: "UVR-MDX-NET-Inst_Main.onnx",
                url: concat!(
                    "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/",
                    "UVR-MDX-NET-Inst_Main.onnx"
                ),
                size: 52_786_726,
                sha256: "8ab401dfe4a548b87deb64f975294bd56ff946aa32903f53b4b24bb13b2cce1e",
                params: MdxParams {
                    n_fft: 5120,
                    hop: 1024,
                    dim_f: 2048,
                    dim_t: 256,
                    compensate: 1.025,
                },
                primary_stem: PrimaryStem::Instrumental,
            },
            ModelId::Best => ModelSpec {
                id: self,
                key: "best",
                file_name: "UVR-MDX-NET-Inst_HQ_3.onnx",
                url: concat!(
                    "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/",
                    "UVR-MDX-NET-Inst_HQ_3.onnx"
                ),
                size: 66_759_214,
                sha256: "317554b07fe1ea5279a77f2b1520a41ea4b93432560c4ffd08792c30fddf9adc",
                params: MdxParams {
                    n_fft: 6144,
                    hop: 1024,
                    dim_f: 3072,
                    dim_t: 256,
                    compensate: 1.022,
                },
                primary_stem: PrimaryStem::Instrumental,
            },
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ModelId::Light => "light",
            ModelId::Balanced => "balanced",
            ModelId::Best => "best",
        }
    }

    /// What the cache names this tier's instrumentals by: the model file, so replacing the model
    /// starts a fresh set instead of serving the old one's.
    pub fn cache_tag(self) -> &'static str {
        let file = self.spec().file_name;
        file.strip_suffix(".onnx").unwrap_or(file)
    }

    pub fn from_key(key: &str) -> Option<ModelId> {
        match key {
            "light" => Some(ModelId::Light),
            "balanced" => Some(ModelId::Balanced),
            "best" => Some(ModelId::Best),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_URL: &str =
        "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/";

    #[test]
    fn test_models() {
        assert_eq!(ModelId::DEFAULT, ModelId::Light);

        let mut keys = std::collections::HashSet::new();
        let mut file_names = std::collections::HashSet::new();

        for &id in &ModelId::ALL {
            let spec = id.spec();
            assert_eq!(spec.id, id);
            assert_eq!(spec.key, id.key());
            assert_eq!(ModelId::from_key(spec.key), Some(id));

            assert!(keys.insert(spec.key), "Duplicate key: {}", spec.key);
            assert!(file_names.insert(spec.file_name), "Duplicate file_name: {}", spec.file_name);
            assert!(
                !id.cache_tag().ends_with(".onnx") && spec.file_name.starts_with(id.cache_tag())
            );

            assert_eq!(spec.sha256.len(), 64);
            assert!(
                spec.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "SHA256 not lowercase hex: {}",
                spec.sha256
            );

            assert!(spec.url.ends_with(spec.file_name));
            assert!(spec.url.starts_with(BASE_URL));
        }

        assert_eq!(ModelId::from_key("unknown"), None);
        // The keys the first pass stored mean nothing now: those fall back to the default.
        assert_eq!(ModelId::from_key("kara"), None);
    }

    /// UVR's model_data.json, for the files above. A wrong n_fft or primary stem doesn't fail
    /// anything else: it just takes out less of the vocals.
    #[test]
    fn params_are_uvrs() {
        let uvr = |id: ModelId| {
            let s = id.spec();
            (s.params.n_fft, s.params.dim_f, s.params.compensate, s.primary_stem)
        };
        assert_eq!(uvr(ModelId::Light), (6144, 2048, 1.035, PrimaryStem::Vocals));
        assert_eq!(uvr(ModelId::Balanced), (5120, 2048, 1.025, PrimaryStem::Instrumental));
        assert_eq!(uvr(ModelId::Best), (6144, 3072, 1.022, PrimaryStem::Instrumental));
        for id in ModelId::ALL {
            assert_eq!((id.spec().params.hop, id.spec().params.dim_t), (1024, 256));
        }
    }
}
