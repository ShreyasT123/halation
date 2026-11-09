use std::{env, path::PathBuf};

fn data_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("Halation")
}

pub(super) fn catalog_path() -> PathBuf {
    data_dir().join("catalog.halation-cat")
}

pub(super) fn cache_root() -> PathBuf {
    data_dir().join("cache")
}
