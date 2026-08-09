use core::str;
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use serde_json::{json, Value};
use std::{collections::HashMap, default, fmt::format, ops::Index, str::FromStr, usize};
use url::form_urlencoded;

use crate::hash;

static ATTR_DEFAULT: &str = "default";
static ATTR_TYPE: &str = "type";
static ATTR_CONV: &str = "conv";
static ATTR_CUT: &str = "cut";
static ATTR_ENCODE: &str = "encode";
static ATTR_PREFIX: &str = "prefix";
static ATTR_SUFFIX: &str = "suffix";
static ATTR_IGNORE: &str = "ignore";

#[derive(Default, Clone, Debug)]
pub struct TheadAttr {
    pub raw: String,
    pub cell_type_raw: String,
    pub cell_type: CellTypes,
    pub cell_enum_type: String,
    pub cell_default: String,
    pub conv: String,
    pub encode: String,
    pub prefix: String,
    pub suffix: String,
    pub ignore: Option<String>,
    pub cut: usize,
}

pub struct NumberParseResult {
    success: bool,
    value: serde_json::Value,
}

pub fn convert_cell_to_json(
    attr: &TheadAttr,
    enum_map: &HashMap<String, HashMap<String, i64>>,
    encode_map: &HashMap<String, String>,
    input: &String,
    epsilon: f64,
) -> (CellTypes, Value) {
    // 1. 添加前后缀
    let spliced = format!("{}{}{}", attr.prefix, input, attr.suffix);
    // 2. 根据conv转换
    let mut cell_string = str_converter(&attr.conv, &spliced);
    // 3. encode 编码
    if attr.encode == "custom" {
        if let Some(ed) = encode_map.get(&cell_string) {
            cell_string = ed.to_string();
        }
    }
    // 4. cut 裁剪
    if attr.cut > 0 {
        cell_string = cell_string[..attr.cut].to_string();
    }

    // 试图将cell的字符串值转数字
    let try_i64 = cell_string.parse::<i64>();
    let try_f64 = cell_string.parse::<f64>();
    let is_string_null = cell_string.to_lowercase() == "null";
    let json_number = if let Ok(i64_val) = try_i64 {
        // 先解析i64
        NumberParseResult {
            success: true,
            value: json_from_i64(i64_val),
        }
    } else if let Ok(f64_col) = try_f64 {
        // 其次解析f64
        NumberParseResult {
            success: true,
            value: json_from_f64(f64_col, epsilon),
        }
    } else {
        NumberParseResult { success: false, value: json!(0) }
    };

    // 根据前面获取的表头可能声明了的类型
    let final_cell_type = if attr.cell_type != CellTypes::Unknown {
        // 如果已经声明类型不为unknown, 则返回已声明类型
        attr.cell_type
    } else if try_i64.is_ok() || try_f64.is_ok() {
        // 如果已声明类型未知, 则尝试解析数字, 若能解析为数字则返回数字类型
        CellTypes::Number
    } else {
        // 默认情况, 字符串类型
        CellTypes::String
    };

    let final_cell_value = if final_cell_type == CellTypes::Enum {
        json!(enum_map.get(&attr.cell_enum_type).and_then(|im| im.get(&cell_string)).unwrap_or(&0))
    } else if cell_string == "" {
        // 如果经过转换后还是空字符串, 则用该列类型对应的默认值填充
        get_default_json_by_cell_type(final_cell_type)
    } else if final_cell_type == CellTypes::String || final_cell_type == CellTypes::NullableString {
        // 字符串类型, 直接填充
        if final_cell_type == CellTypes::NullableString && is_string_null {
            serde_json::Value::Null
        } else {
            json!(cell_string)
        }
    } else if final_cell_type == CellTypes::Bool || final_cell_type == CellTypes::NullableBool {
        if final_cell_type == CellTypes::NullableBool && is_string_null {
            serde_json::Value::Null
        } else {
            json!(match cell_string.as_str() {
                "0" => false,
                s if s.is_empty() => false,
                s if s.to_lowercase() == "true" => true,
                s if s.to_lowercase() == "false" => false,
                _ => true,
            })
        }
    } else if json_number.success || final_cell_type == CellTypes::NullableNumber {
        if final_cell_type == CellTypes::NullableNumber && is_string_null {
            serde_json::Value::Null
        } else {
            json!(&json_number.value)
        }
    } else {
        json!(cell_string)
    };

    return (final_cell_type, final_cell_value);
}

pub fn parse_attr(attr_str: &str, default_attr: &TheadAttr, enum_map: &HashMap<String, HashMap<String, i64>>) -> TheadAttr {
    let hash_map: HashMap<String, String> = form_urlencoded::parse(attr_str.as_bytes()).into_owned().collect();

    let attr_type = hash_map.get(ATTR_TYPE).map_or_else(|| default_attr.cell_type_raw.clone(), |s| s.to_string());
    let attr_cell_type = get_cell_type_by_name(&attr_type, enum_map);

    TheadAttr {
        raw: attr_str.to_string(),
        cell_type_raw: attr_type.to_string(),
        cell_type: attr_cell_type,
        cell_enum_type: if attr_cell_type == CellTypes::Enum { attr_type.to_string() } else { "".to_string() },
        cell_default: hash_map.get(ATTR_DEFAULT).map_or_else(|| default_attr.cell_default.clone(), |s| s.to_string()),
        conv: hash_map.get(ATTR_CONV).map_or_else(|| default_attr.conv.clone(), |s| s.to_string()),
        encode: hash_map.get(ATTR_ENCODE).map_or_else(|| default_attr.encode.clone(), |s| s.to_string()),
        prefix: hash_map.get(ATTR_PREFIX).map_or_else(|| default_attr.prefix.clone(), |s| s.to_string()),
        suffix: hash_map.get(ATTR_SUFFIX).map_or_else(|| default_attr.suffix.clone(), |s| s.to_string()),
        ignore: hash_map.get(ATTR_IGNORE).map_or_else(|| default_attr.ignore.clone(), |s| Some(s.to_string())),
        cut: hash_map.get(ATTR_CUT).unwrap_or(&"0".to_string()).parse::<usize>().unwrap_or(0),
    }
}

pub fn str_converter(conv: &str, s: &str) -> String {
    let raw = s.to_string();
    let conv_insensitive = conv.replace("_", "").to_lowercase();
    match conv_insensitive.as_str() {
        "fnv1a32" => hash::fnv1a32(s.as_bytes()).to_string(),
        "hex" => i64::from_str_radix(s, 16).map_or(raw, |i64_val| i64_val.to_string()),
        "dec" => s.parse::<i64>().map_or(raw, |i64_val| format!("{:X}", i64_val)),
        "camelcase" => s.to_lower_camel_case(),
        "pascalcase" => s.to_upper_camel_case(),
        "snakecase" => s.to_snake_case(),
        _ => raw,
    }
}

pub fn json_from_i64(i: i64) -> Value {
    json!(i)
}

// 将一个float转换为json节点, 如果分数小于epsilon则视为整数
pub fn json_from_f64(f: f64, epsilon: f64) -> Value {
    if f.abs().fract() <= epsilon {
        json!(f as i64)
    } else {
        json!(f)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum CellTypes {
    #[default]
    Unknown,
    String,
    NullableString,
    Number,
    NullableNumber,
    Bool,
    NullableBool,
    Enum,
}

pub fn get_cell_type_by_name(name: &str, enum_map: &HashMap<String, HashMap<String, i64>>) -> CellTypes {
    match name {
        "string" => CellTypes::String,
        "string?" => CellTypes::NullableString,
        "number" => CellTypes::Number,
        "number?" => CellTypes::NullableNumber,
        "bool" => CellTypes::Bool,
        "bool?" => CellTypes::NullableBool,
        s if enum_map.contains_key(s) => CellTypes::Enum,
        _ => CellTypes::Unknown,
    }
}

pub fn get_default_json_by_cell_type(c_type: CellTypes) -> serde_json::Value {
    match c_type {
        CellTypes::String => json!(""),
        CellTypes::NullableString => serde_json::Value::Null,
        CellTypes::Number => json!(0),
        CellTypes::NullableNumber => serde_json::Value::Null,
        CellTypes::Bool => json!(false),
        CellTypes::NullableBool => serde_json::Value::Null,
        _ => json!(""),
    }
}

#[derive(Debug)]
pub struct FuncExpr {
    name: String,
    params: Vec<String>,
}

impl FuncExpr {
    fn parse_at<T: FromStr>(&self, at: usize) -> Option<T> {
        let Some(s) = &self.params.get(at) else {
            return None;
        };
        let Ok(parsed) = s.parse::<T>() else {
            return None;
        };
        Some(parsed)
    }
}

pub fn parse_function(input: &str) -> Option<FuncExpr> {
    if !input.ends_with(')') {
        return None;
    };
    let Some(first_par) = input.find('(') else {
        return None;
    };
    let name = &input[..first_par];
    let params: Vec<String> = input[first_par + 1..input.len() - 1]
        .split(',')
        .map(|p| p.trim().to_string())
        .map(|mut p| {
            if p.starts_with('"') && p.ends_with('"') && p.len() >= 2 {
                p = p[1..p.len() - 1].to_string();
            }
            p
        })
        .filter(|p| !p.is_empty())
        .collect();
    Some(FuncExpr { name: name.to_string(), params })
}
