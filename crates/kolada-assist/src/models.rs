//! The writing models a person may choose, smallest first: what the welcome
//! screen offers and the app downloads into its models folder. Each is
//! checked by its SHA-256 before it is used. The training (tools/train)
//! changes this list when a model is retrained; the app reads it and never
//! hard-codes a model of its own.

/// Where the models are published: a Hugging Face model repository and the
/// revision (a commit, so a file can never change under an app).
pub struct Hub {
    pub repo: &'static str,
    pub revision: &'static str,
}

/// Not published yet: until it is, the app finds a model only where one
/// was put by hand (beside the app, or in its data's models folder).
pub const HUB: Option<Hub> = None;

pub struct Model {
    /// Stable, for the setting that remembers the choice.
    pub id: &'static str,
    /// The file, in the models folder and in the repository. Its name tells
    /// kolada-ai the model was trained for Kolada's fixes (`trained`).
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub const MODELS: &[Model] = &[
    Model { id: "light", file: "notsy-writer-350m-Q4_K_M.gguf", bytes: 229_314_560, sha256: "B4BCDF4A2F3C172F552D0EE16C38506B3A1C3B327589DB430870016D23BD42F8" },
    Model { id: "full", file: "notsy-writer-1.2b-Q4_K_M.gguf", bytes: 730_898_432, sha256: "6F7FD5E9D9878621EA784A53786F2E5617F519118A04B53DD469C33BFF561857" },
];

/// Whether a model file was trained for Kolada's fixes (tools/train), by its
/// name: `Kolada-writer-…` as published, or `notsy-writer-…` from before.
/// Such a model is asked as it was trained (kolada-ai).
pub fn trained(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    ["kolada-writer", "notsy-writer"].iter().any(|p| f.starts_with(p))
}

/// The model a choice names, if it is one.
pub fn by_id(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

/// Where `m` is downloaded from, once the models are published.
pub fn url(m: &Model) -> Option<String> {
    HUB.as_ref().map(|h| format!("https://huggingface.co/{}/resolve/{}/{}", h.repo, h.revision, m.file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_are_smallest_first_and_each_is_whole() {
        assert!(MODELS.windows(2).all(|w| w[0].bytes < w[1].bytes));
        for m in MODELS {
            assert!(trained(m.file) && m.file.ends_with(".gguf"), "{}", m.file);
            assert!(m.sha256.len() == 64 && m.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", m.id);
            assert_eq!(by_id(m.id).map(|f| f.file), Some(m.file));
        }
        assert!(by_id("none").is_none());
    }

    #[test]
    fn a_model_trained_for_kolada_is_known_by_its_name_either_way() {
        for f in ["Kolada-writer-mini-v1.gguf", "kolada-writer-med-v1.gguf", "notsy-writer-1.2b-Q4_K_M.gguf"] {
            assert!(trained(f), "{f}");
        }
        for f in ["LFM2.5-1.2B-Instruct-QAD-Q4_0.gguf", "writer.gguf", "my-kolada-writer.gguf"] {
            assert!(!trained(f), "{f}");
        }
    }

    #[test]
    fn a_url_names_the_file_at_a_fixed_revision() {
        let m = &MODELS[0];
        let url = HUB.as_ref().map(|h| format!("https://huggingface.co/{}/resolve/{}/{}", h.repo, h.revision, m.file));
        assert_eq!(super::url(m), url);
    }
}
