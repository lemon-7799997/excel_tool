use clap::Args;
use std::collections::HashMap;

/// 解析 `KEY=VALUE` 形式的命令行参数
fn parse_key_value(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((key, value)) if !key.trim().is_empty() => Ok((key.trim().to_string(), value.to_string())),
        _ => Err(format!("参数格式应为 KEY=VALUE, 当前传入: `{}`", s)),
    }
}

/// 解析 `NAME=TEXT` 形式的枚举定义, TEXT 中的 `\n` 会被转换为换行
fn parse_enum_value(s: &str) -> Result<(String, String), String> {
    let (name, text) = parse_key_value(s)?;
    Ok((name, text.replace("\\n", "\n")))
}

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

    /// 使用命令行显式传入的参数覆盖当前配置 (toml 配置 / 默认值)。
    ///
    /// 只有显式传入的参数会被覆盖, 未传入 (None / 空列表) 的字段保持 toml 中读取到的值。
    pub fn apply_cli_args(&mut self, args: &ConfigArgs) {
        // 标量配置项: 显式传入则覆盖
        macro_rules! apply_scalar {
            ($($field:ident),* $(,)?) => {
                $(
                    if let Some(value) = &args.$field {
                        self.$field = value.clone();
                    }
                )*
            };
        }

        apply_scalar!(
            mode,
            input,
            output,
            format,
            epsilon,
            zip,
            custom_encode_map_path,
            key_case,
            is_whitelist_mode,
            is_write_meta,
            meta_ext,
            row_transpose_mark,
            row_var_mark,
            row_attr_mark,
        );

        // 映射类配置项: 命令行按 key 覆盖 / 追加到 toml 读取到的表里
        for (key, value) in &args.attrs {
            self.attrs.insert(key.clone(), value.clone());
        }
        for (key, value) in &args.global_key_case {
            self.global_key_case.insert(key.clone(), value.clone());
        }
        for (key, value) in &args.global_attr_by_key {
            self.global_attr_by_key.insert(key.clone(), value.clone());
        }
        for (key, value) in &args.specified_output {
            self.specified_output.insert(key.clone(), value.clone());
        }
        for (key, value) in &args.enums {
            self.enums.insert(key.clone(), value.clone());
        }
    }
}

/// 命令行配置参数: 覆盖 `#excel-tool.settings.toml` 中的同名配置项。
///
/// 所有参数均为可选: 只有显式传入的参数才会覆盖 toml 配置, 未传入的参数保持 toml / 默认值。
/// 布尔参数可写成 `--is-write-meta` (等价于 `--is-write-meta true`) 或 `--is-write-meta false`。
/// 映射类参数 (attr / global-key-case / global-attr-by-key / specified-output / enum) 可重复传入。
#[derive(Args, Debug, Clone, Default)]
pub struct ConfigArgs {
    /// 生成模式 debug/release (debug模式下json会pretty输出)
    #[arg(long, value_name = "MODE")]
    pub mode: Option<String>,

    /// 输入目录
    #[arg(long, value_name = "DIR")]
    pub input: Option<String>,

    /// 输出目录
    #[arg(long, value_name = "DIR")]
    pub output: Option<String>,

    /// 输出格式: txt/json
    ///
    /// 特殊规则:
    /// 根据sheet子表名判断: 如果sheet名明确写了格式名后缀比如: sheet1.txt, 则该sheet生成txt文件
    #[arg(long, value_name = "FORMAT", verbatim_doc_comment)]
    pub format: Option<String>,

    /// 是否写入json字段对应原始 excel 位置的 meta 数据, 仅在 debug 模式下生效
    #[arg(long, value_name = "true|false", num_args = 0..=1, default_missing_value = "true", action = clap::ArgAction::Set)]
    pub is_write_meta: Option<bool>,

    /// meta 数据文件后缀, 会生成 json 字段对应在原始 excel 表格的位置用于 debug
    #[arg(long, value_name = "EXT")]
    pub meta_ext: Option<String>,

    /// Epsilon
    #[arg(long, value_name = "FLOAT")]
    pub epsilon: Option<f64>,

    /// 压缩方法 (仅在 release 模式生效)
    #[arg(long, value_name = "METHOD")]
    pub zip: Option<String>,

    /// 自定义编码表文件的路径
    ///
    /// 要求格式:
    /// 1. txt: 以行分隔每个转换项, 每行以空格/制表符分隔key和value
    /// 会对(attr)行标记了 encode=custom 的列的单元格的字符进行自定义编码转换,
    /// 如果编码表的编码值是数字或可以转为数字的字符串, 则最终生成的json中也是数字
    /// abc => 1 将abc转为1
    /// def => 2 将def转为2
    #[arg(long, value_name = "PATH", verbatim_doc_comment)]
    pub custom_encode_map_path: Option<String>,

    /// transpose转置标记, 只能标记在最左上角单元格
    #[arg(long, value_name = "MARK")]
    pub row_transpose_mark: Option<String>,

    /// 表头 key/var 行标记
    #[arg(long, value_name = "MARK")]
    pub row_var_mark: Option<String>,

    /// 表头属性 (attr) 标记(用于标记默认值, 字段类型等)
    ///
    /// 格式: x-www-form-urlencoded
    /// 示例: type=string&default=abc&conv=PascalCase
    /// 可用attr:
    /// type=number/string/bool     列类型, 可为nullable
    ///    string
    ///    string?
    ///    number
    ///    number?
    ///    bool
    ///    bool?
    /// default=0                   标记默认值, 当单元格为空的时候, 用默认值填充
    /// conv=PascalCase             用于标记列字符串格式化(本配置不区分大小写)
    ///    case转换: PascalCase/camelCase/snake_case
    ///    进制转换：hex/dec
    ///    哈希转换: fnv1a32
    /// prefix=abc                  给本列所有字符加上前缀
    /// suffix=abc                  给本列所有字符加上后缀
    /// encode=custom               给本列所有字符进行编码
    /// ignore=0                    当内容是0时, 将不会把本字段写入json对象
    #[arg(long, value_name = "MARK", verbatim_doc_comment)]
    pub row_attr_mark: Option<String>,

    /// 全局字段名 key 转换格式:
    ///
    /// 不转换: 填空(key_case = "") / 删掉 key_case 行 / 注释掉: # key_case = ""
    /// 转换为小驼峰: key_case = "camelCase"
    /// 转换为大驼峰: key_case = "PascalCase"
    /// 转换为Snake:  key_case = "snake_case"
    /// 默认为不转换
    #[arg(long, value_name = "CASE", verbatim_doc_comment)]
    pub key_case: Option<String>,

    /// 是否开启白名单模式 (toml 示例中未列出, 但属于 Config 支持的配置项)
    #[arg(long, value_name = "true|false", num_args = 0..=1, default_missing_value = "true", action = clap::ArgAction::Set)]
    pub is_whitelist_mode: Option<bool>,

    /// 全局 key 转换表
    ///
    /// "en" = "en" 代表不转换case
    /// 可重复传入, 如: --global-key-case "en=en" --global-key-case "zh_CN=zh_CN"
    #[arg(long = "global-key-case", value_name = "KEY=VALUE", value_parser = parse_key_value, verbatim_doc_comment)]
    pub global_key_case: Vec<(String, String)>,

    /// 根据 key 的格式转换器
    ///
    /// 如果列表中标记的正则匹配到某一列的表头key, 则先默认使用标记的属性(attr), 然后再用该列内标记的属性(attr)覆盖
    /// "name" = "type=string"         当列key是name的时候, 这一列的类型是string
    /// "^id$|.+\\.id$" = "default=0"  当列key是id或者以.id结尾的时候, 这一列默认填充0
    /// 可重复传入, 如: --global-attr-by-key '^id$|.+\\.id$=default=0'
    #[arg(long = "global-attr-by-key", value_name = "REGEX=ATTR", value_parser = parse_key_value, verbatim_doc_comment)]
    pub global_attr_by_key: Vec<(String, String)>,

    /// 指定某个 sheet 的输出路径 (toml 示例中未列出, 但属于 Config 支持的配置项)
    ///
    /// 可重复传入, 如: --specified-output "sheet1=out/sheet1.txt"
    #[arg(long = "specified-output", value_name = "SHEET=PATH", value_parser = parse_key_value)]
    pub specified_output: Vec<(String, String)>,

    /// 按列声明的属性表 (toml 示例中未列出, 但属于 Config 支持的配置项)
    ///
    /// 可重复传入, 如: --attr "name=type=string"
    #[arg(long = "attr", value_name = "KEY=ATTR", value_parser = parse_key_value)]
    pub attrs: Vec<(String, String)>,

    /// 定义枚举, 用多行字符串定义
    ///
    /// 按行解析, 每行一个项, 数字可选, 逗号可选
    /// 命令行中用 \n 表示换行, 可重复传入, 如:
    /// --enum 'Color=Red = 1\nGreen\nYellow\nBlue = 5'
    #[arg(long = "enum", value_name = "NAME=TEXT", value_parser = parse_enum_value, verbatim_doc_comment)]
    pub enums: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML_TEXT: &str = r#"
mode = "debug"
input = "toml_input"
output = "toml_output"
format = "json"
is_write_meta = true
meta_ext = "meta"
[global_key_case]
"en" = "en"
[global_attr_by_key]
"name" = "type=string"
[specified_output]
"sheet1" = "out/sheet1.txt"
[enums]
Color = "Red = 1\nGreen"
"#;

    /// 未传入任何命令行参数时, 配置保持 toml 的值
    #[test]
    fn empty_cli_args_keep_toml_config() {
        let mut cfg: Config = toml::from_str(TOML_TEXT).unwrap();
        cfg.apply_cli_args(&ConfigArgs::default());

        assert_eq!(cfg.mode, "debug");
        assert_eq!(cfg.input, "toml_input");
        assert_eq!(cfg.output, "toml_output");
        assert_eq!(cfg.format, "json");
        assert!(cfg.is_write_meta);
        assert_eq!(cfg.meta_ext, "meta");
        assert_eq!(cfg.global_key_case.get("en").map(String::as_str), Some("en"));
        assert_eq!(cfg.global_attr_by_key.get("name").map(String::as_str), Some("type=string"));
        assert_eq!(cfg.specified_output.get("sheet1").map(String::as_str), Some("out/sheet1.txt"));
        assert_eq!(cfg.enums.get("Color").map(String::as_str), Some("Red = 1\nGreen"));
    }

    /// 显式传入的命令行参数覆盖 toml, 未传入的字段保持不变
    #[test]
    fn cli_args_override_toml_config() {
        let mut cfg: Config = toml::from_str(TOML_TEXT).unwrap();
        let args = ConfigArgs {
            mode: Some("release".to_string()),
            input: Some("cli_input".to_string()),
            format: Some("txt".to_string()),
            epsilon: Some(1e-9),
            is_write_meta: Some(false),
            key_case: Some("camelCase".to_string()),
            global_key_case: vec![("en".to_string(), "EN".to_string()), ("jp".to_string(), "jp".to_string())],
            ..Default::default()
        };
        cfg.apply_cli_args(&args);

        // 显式传入 -> 覆盖
        assert_eq!(cfg.mode, "release");
        assert_eq!(cfg.input, "cli_input");
        assert_eq!(cfg.format, "txt");
        assert_eq!(cfg.epsilon, 1e-9);
        assert!(!cfg.is_write_meta);
        assert_eq!(cfg.key_case, "camelCase");

        // 未传入 -> 保持 toml
        assert_eq!(cfg.output, "toml_output");
        assert_eq!(cfg.meta_ext, "meta");
        assert_eq!(cfg.specified_output.get("sheet1").map(String::as_str), Some("out/sheet1.txt"));

        // 映射类: 同名 key 覆盖, 新 key 追加
        assert_eq!(cfg.global_key_case.get("en").map(String::as_str), Some("EN"));
        assert_eq!(cfg.global_key_case.get("jp").map(String::as_str), Some("jp"));
    }

    /// 命令行枚举值中的 `\n` 会被还原成换行
    #[test]
    fn enum_arg_restores_newlines() {
        let (name, text) = parse_enum_value("Color=Red = 1\\nGreen\\nYellow").unwrap();
        assert_eq!(name, "Color");
        assert_eq!(text, "Red = 1\nGreen\nYellow");
    }
}
