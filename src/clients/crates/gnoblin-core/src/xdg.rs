use std::path::PathBuf;

pub fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".local/share"))
                .filter(|p| p.is_absolute())
        })
}

pub fn data_dirs() -> Vec<PathBuf> {
    let Some(raw_dirs) = std::env::var_os("XDG_DATA_DIRS").filter(|s| !s.is_empty()) else {
        return default_data_dirs();
    };
    let dirs: Vec<PathBuf> = std::env::split_paths(&raw_dirs)
        .filter(|path| path.is_absolute())
        .collect();
    if dirs.is_empty() {
        default_data_dirs()
    } else {
        dirs
    }
}

fn default_data_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/local/share"),
        PathBuf::from("/usr/share"),
    ]
}
