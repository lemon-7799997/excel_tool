#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]

mod cell_helper;
mod custom_encode;
mod enum_helper;
mod excel_helper;
mod flatten;
mod hash;
mod path_helper;
mod result_helper;
mod vec_helper;
use cell_helper::CellTypes;
use cell_helper::TheadAttr;

use calamine::{Cell, DataType};
use colored::*;
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use path_clean::PathClean;
use rayon::{join, prelude::*, vec};
use serde::Deserialize;
use serde_json::{json, Number, Value};
use std::clone;
use std::collections::HashMap;
use std::default;
use std::error::Error;
use std::fmt::format;
use std::fs::DirEntry;
use std::fs::File;
use std::hash::Hash;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::usize;
use std::{env, fs, io, thread};
use url::form_urlencoded;

use crate::result_helper::ContentKinds;
use crate::result_helper::SheetColumn;

include!("config.rs");

fn is_in_whitelist(str: &str) -> bool {
    str == "*"
}

fn is_in_blacklist(str: &str) -> bool {
    str.starts_with("#") || str.starts_with("~")
}

fn remove_after(str: &str, after: &str) -> String {
    let mut result = str.to_string();
    if let Some(pos) = str.find(after) {
        result = str[..pos].to_string();
    }
    return result;
}

fn filter_map_file<F>(entry: DirEntry, predicate: F) -> Option<PathBuf>
where
    F: Fn(&str) -> bool,
{
    let path = entry.path();
    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
        if predicate(name) {
            return Some(path);
        }
    }
    None
}

pub fn ensure_path(base_dir: &Path, x: &str) -> std::io::Result<Option<PathBuf>> {
    if x.is_empty() {
        return Ok(None);
    }
    let path = base_dir.join(x).clean();
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::File::create(&path)?;
    }
    Ok(Some(path))
}
static DEBUG: AtomicBool = AtomicBool::new(false);

macro_rules! debug_print_head {
    ($($arg:tt)*) => {
        if DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
            println!("[DEBUG] > {}", format_args!($($arg)*));
        }
    };
}
macro_rules! debug_print {
    ($($arg:tt)*) => {
        if DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
            println!($($arg)*);
        }
    };
}

fn read_excel(
    excel_path: &PathBuf,
    base_path: &PathBuf,
    cfg: &Config,
    enums_map: &HashMap<String, HashMap<String, i64>>,
    encode_map: &HashMap<String, String>,
) -> Result<Vec<result_helper::SheetResult>, Box<dyn Error>> {
    let excel_name = excel_path.file_name().and_then(|f| f.to_str()).unwrap_or("");

    let sheets = excel_helper::read_cells(excel_path)?;

    // 创建默认attr头
    let default_attr = TheadAttr::default();

    // sheet内容与错误报告
    let mut reports: Vec<result_helper::SheetResult> = vec![];

    // 从toml配置文件读取的: 根据key名转换cell的attr规则
    let global_attr_by_key_list: Vec<(Option<regex::Regex>, TheadAttr)> = cfg
        .global_attr_by_key
        .iter()
        .map(|(k, v)| (regex::Regex::new(k).ok(), cell_helper::parse_attr(v, &default_attr, enums_map)))
        .collect();

    let find_global_attr_by_key = |key: &str| -> Option<&TheadAttr> {
        for (re, value) in &global_attr_by_key_list {
            if let Some(re) = re {
                if re.is_match(key) {
                    return Some(value);
                }
            }
        }
        None
    };

    let cfg_content_kind = result_helper::ContentKinds::from(&cfg.format);

    for sheet in sheets {
        let write_sheet_name = remove_after(&sheet.name, "#");

        let reading_at = format!("{}/{}", excel_name, &sheet.name);

        if write_sheet_name.is_empty() || is_in_blacklist(&write_sheet_name) {
            println!("> ⏩ 跳过: {}", &reading_at);
            continue;
        }

        reports.push(result_helper::SheetResult::new(excel_name, &write_sheet_name, &sheet.name));
        let report_sheet = reports.last_mut().unwrap();

        let Some(all_rows) = &sheet
            .rows
            .get(..)
            .map(|data| data.iter().map(|r| excel_helper::data_vec_to_string_vec(&r.iter().collect())).collect::<Vec<Vec<String>>>())
        else {
            continue;
        };

        let Some(first_row) = all_rows.get(0) else {
            report_sheet.warn(format!("未找到有效行: {reading_at}"));
            continue;
        };

        debug_print_head!("reading:  `{}`  rows: {}", reading_at, all_rows.len());

        let is_cell_ok = |s: &str| -> bool {
            if is_in_blacklist(s) {
                return false;
            }
            // 白名单过滤
            if cfg.is_whitelist_mode && !is_in_whitelist(s) {
                return false;
            }
            return true;
        };

        let find_head_row = |mark: &String| -> Option<&Vec<String>> { all_rows.iter().find(|row| row.first().is_some_and(|s| s == mark)) };

        let Some(thead_var_strings) = find_head_row(&cfg.row_var_mark) else {
            report_sheet.warn(format!("未找到 var 行"));
            continue;
        };
        if thead_var_strings.len() == 0 {
            continue;
        }

        let thead_attr_strings = find_head_row(&cfg.row_attr_mark).unwrap_or(&vec![]).clone();

        debug_print!("  var {:?}", thead_var_strings);
        debug_print!("  attr {:?}", thead_attr_strings);

        let mut table_result: Vec<SheetColumn> = vec![];

        let mut valid_columns: Vec<bool> = Vec::with_capacity(thead_var_strings.len());
        valid_columns.resize(thead_var_strings.len(), false);

        // 开始遍历表格每一行
        for (row_i, row) in all_rows.iter().enumerate() {
            // 获取每行第一个元素, 如果第一个元素是特定字符, 这行被过滤掉
            if !matches!(row.first(), Some(row_first_cell) if is_cell_ok(row_first_cell)) {
                continue;
            }
            // 从toml配置读取的根据key定制的attr, 如果读取不到, 则用前面创建的默认attr
            let mut auto_index_map: HashMap<String, usize> = HashMap::new();
            for (key_i, key_raw) in thead_var_strings.iter().enumerate() {
                if !matches!(first_row.get(key_i), Some(cell) if is_cell_ok(cell)) {
                    continue;
                }
                if !matches!(thead_var_strings.get(key_i) , Some(cell) if !cell.is_empty() && is_cell_ok(cell)) {
                    continue;
                }

                valid_columns[key_i] = true;

                let mut flat_key = flatten::FlattenKey::new(key_raw.to_string(), |sub_key| {
                    cell_helper::str_converter(cfg.global_key_case.get(sub_key).unwrap_or(&cfg.key_case), sub_key)
                });
                match flatten::fill_auto_indexes(&mut flat_key, &mut auto_index_map) {
                    more_than if more_than > 1 => {
                        report_sheet.warn(format!("`{key_raw}` 有{more_than}个自动索引, 但只有最后一个会被识别, 请明确指定索引值"));
                    }
                    _ => {}
                };
                let def_attr = find_global_attr_by_key(key_raw).unwrap_or(&default_attr);
                let temp_col = SheetColumn::new(
                    key_i,
                    flat_key,
                    cell_helper::parse_attr(
                        thead_attr_strings.get(key_i).unwrap_or(&&"".to_string()),
                        find_global_attr_by_key(key_raw).unwrap_or(&default_attr),
                        enums_map,
                    ),
                );
                table_result.push(temp_col);
            }

            // 整行都为空的表格行直接跳过
            if row.iter().all(|cell| cell.is_empty()) {
                continue;
            }

            // 数据行
            for (col_j, col) in row.iter().enumerate() {
                if matches!(valid_columns.get(col_j), Some(false)) {
                    continue;
                }

                let target_table_row = match table_result.iter_mut().find(|c1| c1.column == col_j) {
                    Some(v) => v,
                    None => continue,
                };
                // 获取attr头
                let thead_attr = &mut target_table_row.attr;
                // 如果当前单元格是空则用attr标注的默认值填充, 如果attr没有标注默认值则用对应类型的默认值填充(number为0, 其他为空字符串)
                let cell_content = if col.is_empty() { &thead_attr.cell_default } else { &col.to_string() };

                let (final_cell_type, cell_json) = cell_helper::convert_cell_to_json(thead_attr, enums_map, encode_map, cell_content, cfg.epsilon);

                // 根据前面获取的表头可能声明了的类型
                if thead_attr.cell_type == CellTypes::Unknown {
                    thead_attr.cell_type = final_cell_type
                }

                target_table_row.body.push(cell_json);
                target_table_row.meta.push((row_i + 1).to_string());
            }
        }

        let mut final_output_path = if let Some(spec_output) = cfg.specified_output.get(&write_sheet_name) {
            path_helper::path_normalize(&base_path.join(spec_output))
        } else {
            path_helper::path_normalize(&base_path.join(&cfg.output).join(&write_sheet_name))
        };

        let content_kind = if let Some(st_ext) = final_output_path.extension() {
            result_helper::ContentKinds::from(st_ext.to_str().unwrap_or(""))
        } else {
            final_output_path.set_extension(cfg_content_kind.as_str());
            cfg_content_kind
        };

        let body_result = match content_kind {
            ContentKinds::Json => result_helper::get_json_list_by_table(&table_result),
            ContentKinds::Csv => result_helper::get_table_list_by_table(&table_result, ","),
            _ => result_helper::get_table_list_by_table(&table_result, "\t"),
        };

        report_sheet.finish_sheet(
            content_kind,
            final_output_path,
            thead_var_strings.iter().filter(|s| is_cell_ok(s)).map(|s| s.clone()).collect(),
            thead_attr_strings.iter().filter(|s| is_cell_ok(s)).map(|s| s.clone()).collect(),
            body_result.0,
            body_result.1,
        );
    }
    Ok(reports)
}

fn base_dir() -> io::Result<PathBuf> {
    // 检测是否是 cargo run 运行
    if env::var("CARGO").is_ok() {
        // cargo run 下直接用当前工作目录
        env::current_dir().map_err(|e| io::Error::new(e.kind(), format!("> ❌ 无法获取当前工作目录: {}", e)))
    } else {
        // 普通 exe 运行，用 exe 所在目录
        let exe_path = env::current_exe().map_err(|e| io::Error::new(e.kind(), format!("> ❌ 无法获取当前可执行文件路径: {}", e)))?;

        let exe_dir = exe_path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, format!("> ❌ 路径中没有父目录: {}", exe_path.display())))?
            .to_path_buf();

        Ok(exe_dir)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("=====================================");
    println!("> Excel Tool v1.0.0");
    println!("> Author: Lemon");
    println!("> Powered by Rust 1.96 & OpenAI GPT-5");
    println!("=====================================");

    let args: Vec<String> = env::args().skip(1).collect();

    let base_dir = base_dir()?;

    let config_path = if let Some(arg0) = args.get(0) {
        let arg0_path = Path::new(arg0);
        if arg0_path.is_absolute() {
            arg0_path.to_path_buf()
        } else {
            base_dir.join(arg0)
        }
    } else {
        base_dir.join("#excel-tool.settings.toml")
    };

    let debug = std::env::args().any(|x| x == "debug");
    DEBUG.store(debug, std::sync::atomic::Ordering::Relaxed);

    // 读取生成工具配置
    let config_content = if Path::new(&config_path).exists() {
        fs::read_to_string(&config_path)?
    } else {
        println!("> 📄 未找到配置: {:?}, 将使用默认配置", &config_path);
        String::new()
    };

    let cfg: Config = toml::from_str::<Config>(&config_content).expect("> ❌ 配置: 解析配置文件内容失败");

    let enums_map: HashMap<String, HashMap<String, i64>> = cfg.enums.clone().into_iter().map(|(name, text)| (name, enum_helper::parse_enum_type(&text))).collect();

    let is_release = cfg.mode == "release";

    println!();
    println!("> 📄 读取配置: {:?}", &config_path);
    debug_print!("{cfg:?}");

    // 解析输入路径
    let input_path = base_dir.join(&cfg.input).canonicalize().expect("> ❌ 配置: 输入路径 `input` 不合法");

    println!();
    println!("> 📂 输入目录: {}", path_helper::path_colored(&input_path));

    // 解析自定义编码表
    let mut custom_encode_map: HashMap<String, String> = HashMap::new();

    if &cfg.custom_encode_map_path != "" {
        let target_path = &base_dir
            .join(&cfg.custom_encode_map_path)
            .canonicalize()
            .expect(&format!("> ⚠️ 未找到生成工具配置文件所定义的编码表: {}", &cfg.custom_encode_map_path));
        match custom_encode::get_encode_map(target_path) {
            Ok(map) => custom_encode_map = map,
            Err(dec_err) => {
                println!("> ⚠️ 配置文件中定义了编码表路径: {:?}, 但是解析编码表失败: {}", target_path, dec_err);
            }
        }
    }

    let input_files = fs::read_dir(&input_path)?.filter_map(Result::ok);

    // 读取所有.xlsx文件
    let excel_files: Vec<PathBuf> = input_files.filter_map(|e| filter_map_file(e, |n| !is_in_blacklist(n) && n.ends_with(".xlsx"))).collect();

    println!("> 📄 处理 {} 个 excel 文件", excel_files.len());

    let execute_start = Instant::now(); // 记录开始时间

    let reports = excel_files
        .par_iter()
        .filter_map(|excel_path| match read_excel(excel_path, &base_dir, &cfg, &enums_map, &custom_encode_map) {
            Ok(report) => Some(report),
            Err(err) => {
                eprintln!("    > ❌ 读取 {:?} 文件出错: {:?}", &excel_path, err);
                None
            }
        })
        .flatten()
        .collect::<Vec<_>>();

    let mut warn_count: usize = 0;

    for w in reports.iter().map(|s| &s.warnings).flatten() {
        println!("  > ⚠️ {}", &w.yellow());
        warn_count += 1;
    }

    println!();
    println!();

    result_helper::write_sheets(reports, &cfg)?;

    println!();
    println!("{}", format!("> 🕒 共用时: {:.6} 秒", execute_start.elapsed().as_secs_f64()).cyan());

    println!();
    if std::env::args().any(|x| x == "--wait") && warn_count > 0 {
        println!("⚠️ 遇到 {} 个警告, 按下回车退出程序...", warn_count);
        println!();
        let mut buffer = String::new();
        io::stdin().read_line(&mut buffer)?;
    }

    Ok(())
}
