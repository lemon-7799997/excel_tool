use std::collections::HashMap;

fn get_default_mode() -> String {
    "debug".to_string()
}

fn get_default_path() -> String {
    "./".to_string()
}

fn get_default_zip() -> String {
    "gzip".to_string()
}

fn get_default_whitelist_mark() -> String {
    "*".to_string()
}

fn get_default_meta_ext() -> String {
    "meta".to_string()
}

fn get_default_row_transpose_mark() -> String {
    "##transpose".to_string()
}

fn get_default_row_var_mark() -> String {
    "##var".to_string()
}

fn get_default_row_attr_mark() -> String {
    "##attr".to_string()
}

fn get_default_format() -> String {
    "json".to_string()
}

fn get_default_mark_count() -> usize {
    2
}

fn get_default_thead_attrs_at() -> usize {
    0
}

fn get_default_thead_keys_at() -> usize {
    1
}

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    #[serde(default = "get_default_mode")]
    pub mode: String,

    #[serde(default = "get_default_path")]
    pub input: String,

    #[serde(default = "get_default_path")]
    pub output: String,

    #[serde(default = "get_default_format")]
    pub format: String,

    #[serde(default)]
    pub epsilon: f64,

    #[serde(default = "get_default_zip")]
    pub zip: String,

    #[serde(default)]
    pub custom_encode_map_path: String,

    #[serde(default)]
    pub key_case: String,

    #[serde(default)]
    pub is_whitelist_mode: bool,

    #[serde(default)]
    pub is_write_meta: bool,

    #[serde(default = "get_default_meta_ext")]
    pub meta_ext: String,

    #[serde(default = "get_default_row_transpose_mark")]
    pub row_transpose_mark: String,

    #[serde(default = "get_default_row_var_mark")]
    pub row_var_mark: String,

    #[serde(default = "get_default_row_attr_mark")]
    pub row_attr_mark: String,

    #[serde(default)]
    pub attrs: HashMap<String, String>,

    #[serde(default)]
    pub global_key_case: HashMap<String, String>,

    #[serde(default)]
    pub global_attr_by_key: HashMap<String, String>,

    #[serde(default)]
    pub specified_output: HashMap<String, String>,

    #[serde(default)]
    pub enums: HashMap<String, String>,
}

impl Config {
    pub fn is_debug(&self) -> bool {
        self.mode == "debug"
    }
    pub fn is_release(&self) -> bool {
        self.mode == "release"
    }
}
