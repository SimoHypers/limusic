use crate::mdx::MdxParams;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelId {
    Kara,
    Kara2,
    InstHq3,
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
    pub params: MdxParams,
    pub primary_stem: PrimaryStem,
}

impl ModelId {
    pub const ALL: [ModelId; 3] = [ModelId::Kara, ModelId::Kara2, ModelId::InstHq3];
    pub const DEFAULT: ModelId = ModelId::Kara;

    pub fn spec(self) -> ModelSpec {
        match self {
            ModelId::Kara => ModelSpec {
                id: ModelId::Kara,
                key: "kara",
                file_name: "UVR_MDXNET_KARA.onnx",
                url: concat!(
                    "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/",
                    "UVR_MDXNET_KARA.onnx"
                ),
                size: 29_704_436,
                sha256: "e3167c87333a48548413e972a286bf40bf5694001d2853861eb1435953f02d63",
                params: MdxParams {
                    n_fft: 5120,
                    hop: 1024,
                    dim_f: 2048,
                    dim_t: 256,
                    compensate: 1.065,
                },
                primary_stem: PrimaryStem::Vocals,
            },
            ModelId::Kara2 => ModelSpec {
                id: ModelId::Kara2,
                key: "kara_2",
                file_name: "UVR_MDXNET_KARA_2.onnx",
                url: concat!(
                    "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/",
                    "UVR_MDXNET_KARA_2.onnx"
                ),
                size: 52_786_726,
                sha256: "bf32e15105a09c0f7dddd2b67346146334d6f3ecb399ed7638eba2ab07cbf5f4",
                params: MdxParams {
                    n_fft: 5120,
                    hop: 1024,
                    dim_f: 2048,
                    dim_t: 256,
                    compensate: 1.065,
                },
                primary_stem: PrimaryStem::Instrumental,
            },
            ModelId::InstHq3 => ModelSpec {
                id: ModelId::InstHq3,
                key: "inst_hq_3",
                file_name: "UVR-MDX-NET-Inst_HQ_3.onnx",
                url: concat!(
                    "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/",
                    "UVR-MDX-NET-Inst_HQ_3.onnx"
                ),
                size: 66_759_214,
                sha256: "317554b07fe1ea5279a77f2b1520a41ea4b93432560c4ffd08792c30fddf9adc",
                params: MdxParams::default(),
                primary_stem: PrimaryStem::Instrumental,
            },
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ModelId::Kara => "kara",
            ModelId::Kara2 => "kara_2",
            ModelId::InstHq3 => "inst_hq_3",
        }
    }

    pub fn from_key(key: &str) -> Option<ModelId> {
        match key {
            "kara" => Some(ModelId::Kara),
            "kara_2" => Some(ModelId::Kara2),
            "inst_hq_3" => Some(ModelId::InstHq3),
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
        assert_eq!(ModelId::DEFAULT, ModelId::Kara);

        let mut keys = std::collections::HashSet::new();
        let mut file_names = std::collections::HashSet::new();

        for &id in &ModelId::ALL {
            let spec = id.spec();
            assert_eq!(spec.id, id);
            assert_eq!(spec.key, id.key());
            assert_eq!(ModelId::from_key(spec.key), Some(id));
            match id {
                ModelId::Kara => assert_eq!(spec.primary_stem, PrimaryStem::Vocals),
                ModelId::Kara2 => assert_eq!(spec.primary_stem, PrimaryStem::Instrumental),
                ModelId::InstHq3 => assert_eq!(spec.primary_stem, PrimaryStem::Instrumental),
            }

            assert!(keys.insert(spec.key), "Duplicate key: {}", spec.key);
            assert!(
                file_names.insert(spec.file_name),
                "Duplicate file_name: {}",
                spec.file_name
            );

            assert_eq!(spec.sha256.len(), 64);
            assert!(
                spec.sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "SHA256 not lowercase hex: {}",
                spec.sha256
            );

            assert!(spec.url.ends_with(spec.file_name));
            assert!(spec.url.starts_with(BASE_URL));

            if id == ModelId::InstHq3 {
                let def = MdxParams::default();
                assert_eq!(spec.params.n_fft, def.n_fft);
                assert_eq!(spec.params.hop, def.hop);
                assert_eq!(spec.params.dim_f, def.dim_f);
                assert_eq!(spec.params.dim_t, def.dim_t);
                assert_eq!(spec.params.compensate, def.compensate);
            }
        }

        assert_eq!(ModelId::from_key("unknown"), None);
    }
}
