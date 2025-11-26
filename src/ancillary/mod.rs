use std::{
    env,
    path::PathBuf
};

pub fn tempfiles_location() -> PathBuf {
    let mut temp_dir = env::temp_dir();
    temp_dir.push("extract_text_from_file");
    temp_dir
}
