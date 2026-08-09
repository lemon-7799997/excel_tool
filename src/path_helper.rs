use colored::*;
use std::path::{Path, PathBuf};

use colored::ColoredString;

/// 简单的路径规范化：去掉 "." 和处理 ".."
pub fn path_normalize(path: &Path) -> PathBuf {
    let mut components = vec![];
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            other => components.push(other.as_os_str()),
        }
    }
    components.iter().collect()
}

pub fn path_colored<P>(p: P) -> ColoredString
where
    P: AsRef<Path>,
{
    p.as_ref().to_string_lossy().to_string().blue()
}
