use colored::*;
use flate2::{write::GzEncoder, Compression};
use std::{
    cmp,
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use serde_json::{json, Value};

use crate::{cell_helper::TheadAttr, excel_helper, flatten, path_helper, Config};

#[derive(Clone, Debug)]
pub struct SheetColumn {
    pub column: usize,
    pub column_name: String,
    pub key: flatten::FlattenKey,
    pub body: Vec<serde_json::Value>,
    pub meta: Vec<String>,
    pub attr: TheadAttr,
}

impl SheetColumn {
    pub fn new(column: usize, key: flatten::FlattenKey, attr: TheadAttr) -> Self {
        SheetColumn {
            column,
            column_name: excel_helper::num_to_col(column + 1),
            key,
            body: vec![],
            meta: vec![],
            attr,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum ContentKinds {
    #[default]
    Json,
    Txt,
    Csv,
}

impl ContentKinds {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentKinds::Json => "json",
            ContentKinds::Txt => "txt",
            ContentKinds::Csv => "csv",
        }
    }
    pub fn from(str: &str) -> Self {
        match str {
            "json" => ContentKinds::Json,
            "csv" => ContentKinds::Csv,
            _ => ContentKinds::Txt,
        }
    }
}

pub fn get_json_list_by_table(list: &Vec<SheetColumn>) -> (Vec<Value>, Vec<String>) {
    if list.len() == 0 {
        return (vec![], vec![]);
    }
    let len: usize = list.iter().map(|c| c.body.len()).max().unwrap_or(0);
    let mut result: Vec<Value> = vec![];
    let mut meta: Vec<String> = vec![];

    for row_i in 0..len {
        let mut json_obj: Value = Value::Object(serde_json::Map::new());
        for col in list {
            let Some(cell) = col.body.get(row_i) else {
                continue;
            };
            if let Some(ig) = &col.attr.ignore {
                if ig == &cell.to_string() {
                    continue;
                }
            }
            flatten::set_json_value(&mut json_obj, &col.key.parts, cell.clone());
            meta.push(format!(
                "$[{row_i}].{} {}:{}",
                &col.key.to_display_string(),
                col.column_name,
                col.meta.get(row_i).unwrap_or(&"".to_string())
            ));
        }
        result.push(json_obj);
    }
    return (result, meta);
}

pub fn get_table_list_by_table(list: &Vec<SheetColumn>, splitter: &str) -> (Vec<Value>, Vec<String>) {
    if list.len() == 0 {
        return (vec![], vec![]);
    }
    let len: usize = list.iter().map(|c| c.body.len()).max().unwrap_or(0);
    let mut result: Vec<Value> = vec![];
    let meta: Vec<String> = vec![];
    for row_i in 0..len {
        let mut row = String::new();
        for (col_i, col) in list.iter().enumerate() {
            let Some(cell) = col.body.get(row_i) else {
                continue;
            };
            let s = match cell {
                Value::String(s) => s.clone(),
                _ => cell.to_string(),
            } + if col_i != list.len() - 1 { splitter } else { "" };
            row.push_str(&s);
        }
        result.push(json!(row));
    }
    return (result, meta);
}

#[derive(Debug, Clone)]
pub struct SheetResult {
    pub excel_name: String,
    pub name: String,
    pub kind: ContentKinds,
    pub warnings: Vec<String>,
    pub full_name_raw: String,
    pub full_name: String,
    pub output_path: PathBuf,
    pub output_path_string: String,
    pub is_finished: bool,
    pub attr_strings: Vec<String>,
    pub var_strings: Vec<String>,
    pub rows: Vec<Value>,
    pub meta: Vec<String>,
}

impl SheetResult {
    pub fn new(excel_name: &str, sheet_name: &str, sheet_name_raw: &str) -> SheetResult {
        SheetResult {
            excel_name: excel_name.to_string(),
            name: sheet_name.to_string(),
            kind: ContentKinds::Json,
            warnings: vec![],
            full_name_raw: format!("{excel_name}/{sheet_name_raw}"),
            full_name: format!("{excel_name}/{sheet_name}"),
            output_path: PathBuf::new(),
            output_path_string: "".to_string(),
            is_finished: false,
            attr_strings: vec![],
            var_strings: vec![],
            rows: vec![],
            meta: vec![],
        }
    }

    pub fn merge_from(&mut self, other: &mut SheetResult) {
        self.rows.append(&mut other.rows);
    }

    pub fn warn(&mut self, info: String) {
        self.warnings.push(format!("{}: {}", self.full_name, info));
    }

    pub fn warn_row(&mut self, row: usize, info: String) {
        self.warnings.push(format!("[{row}] {info}"));
    }

    pub fn warn_cell(&mut self, row: usize, col: usize, cell: String, info: String) {
        self.warnings.push(format!("[{}:{row}]「  {cell}  」{info}", excel_helper::num_to_col(col)));
    }

    pub fn finish_sheet(&mut self, kind: ContentKinds, output_path: PathBuf, var_strings: Vec<String>, attr_strings: Vec<String>, body: Vec<Value>, meta: Vec<String>) {
        self.is_finished = true;
        self.kind = kind;
        self.output_path = output_path;
        self.output_path_string = self.output_path.to_string_lossy().to_string();
        self.attr_strings = attr_strings;
        self.var_strings = var_strings;
        self.rows = body;
        self.meta = meta;
    }
}

pub fn write_sheets(mut reports: Vec<SheetResult>, cfg: &Config) -> anyhow::Result<()> {
    let mut name_wid_max = 0;

    let mut final_write_map: HashMap<String, SheetResult> = HashMap::new();

    for sheet in reports.iter_mut() {
        name_wid_max = std::cmp::max(name_wid_max, sheet.full_name.len());
        if let Some(merge_to) = final_write_map.get_mut(&sheet.output_path_string) {
            merge_to.merge_from(sheet);
            println!("{}", format!("> 🔄 合并: {} <-> {}", merge_to.full_name_raw, sheet.full_name_raw).bright_blue());
        } else {
            final_write_map.insert(sheet.output_path_string.to_string(), sheet.clone());
        };
    }

    let is_debug = cfg.is_debug();
    let is_release = cfg.is_release();

    for (_, sheet) in &final_write_map {
        let SheetResult {
            output_path: op_path,
            output_path_string: op_path_str,
            full_name: op_name,
            is_finished: is_fin,
            ..
        } = sheet;

        if !is_fin {
            println!("{}", format!("> ⚠️ 跳过未完成表格: {}", op_name).yellow());
            continue;
        }

        let create_fail = "> ❌ 输出目录创建失败";
        let Some(op_dir) = op_path.parent() else {
            return Err(anyhow::anyhow!(create_fail));
        };
        if !op_dir.exists() {
            fs::create_dir_all(op_dir).expect(create_fail);
            println!("> ✅ 输出目录已创建: {}", path_helper::path_colored(&op_dir));
        }

        let final_list = &sheet.rows[..];
        let lc = final_list.len();
        println!("> ✅ {}", format!("生成 {lc:^4} 行: {op_name:<name_wid_max$}-> {op_path_str}").green());

        let final_string = if ContentKinds::Json == sheet.kind {
            if is_release {
                serde_json::to_string(&final_list)?
            } else {
                serde_json::to_string_pretty(&final_list)?
            }
        } else {
            let splitter = match sheet.kind {
                ContentKinds::Csv => ",",
                _ => "\t",
            };
            let mut final_body = final_list.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect::<Vec<String>>();
            final_body.insert(0, sheet.var_strings.join(splitter));
            final_body.join("\n")
        };

        let final_write_data = match cfg.zip.as_str() {
            "gzip" if is_release => gzip_compress(final_string.as_bytes()),
            _ => final_string.into_bytes(),
        };

        if is_debug && cfg.is_write_meta {
            let meta_string = sheet.full_name.to_string() + "\n" + &sheet.meta.join("\n");
            let stem = op_path.file_stem().unwrap_or(op_path.as_os_str());
            let new_name = format!("~{}", stem.to_string_lossy());
            fs::write(op_path.with_file_name(new_name).with_extension(&cfg.meta_ext), meta_string)?
        }

        fs::write(&op_path, final_write_data)?
    }

    Ok(())
}

pub fn gzip_compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}
