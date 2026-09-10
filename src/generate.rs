use colored::*;
use flate2::{write::GzEncoder, Compression};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    cmp,
    collections::HashMap,
    env,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use serde_json::{json, Value};

use crate::{
    cell_helper::{self, CellTypes, TheadAttr},
    config::Config,
    custom_encode, enum_helper, excel_helper, flatten, path_helper, Cli,
};

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

fn read_excel(
    excel_path: &PathBuf,
    base_path: &PathBuf,
    cfg: &Config,
    enums_map: &HashMap<String, HashMap<String, i64>>,
    encode_map: &HashMap<String, String>,
) -> Result<Vec<SheetResult>, Box<dyn std::error::Error>> {
    let excel_name = excel_path.file_name().and_then(|f| f.to_str()).unwrap_or("");

    let sheets = excel_helper::read_cells(excel_path)?;

    // 创建默认attr头
    let default_attr = TheadAttr::default();

    // sheet内容与错误报告
    let mut reports: Vec<SheetResult> = vec![];

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

    let cfg_content_kind = ContentKinds::from(&cfg.format);

    for sheet in sheets {
        let write_sheet_name = remove_after(&sheet.name, "#");

        let reading_at = format!("{}/{}", excel_name, &sheet.name);

        if write_sheet_name.is_empty() || is_in_blacklist(&write_sheet_name) {
            println!("> ⏩ 跳过: {}", &reading_at);
            continue;
        }

        reports.push(SheetResult::new(excel_name, &write_sheet_name, &sheet.name));
        let report_sheet = reports.last_mut().unwrap();

        let Some(first_cell) = sheet.rows.first().and_then(|v| v.first()) else {
            continue;
        };

        let is_transposed: bool = first_cell.to_string() == cfg.row_transpose_mark;

        let Some(all_rows) = (if is_transposed {
            &sheet.rows.get(..).map(|data| {
                let rows_data: Vec<Vec<String>> = data.iter().map(|r| excel_helper::data_vec_to_string_vec(&r.iter().collect())).collect();
                if rows_data.is_empty() {
                    return Vec::new();
                }
                let max_cols = rows_data.iter().map(|row| row.len()).max().unwrap_or(0);
                // 转置，缺失的值用空字符串填充
                (0..max_cols)
                    .map(|col_idx| rows_data.iter().map(|row| row.get(col_idx).unwrap_or(&String::new()).clone()).collect::<Vec<String>>())
                    .collect::<Vec<Vec<String>>>()
            })
        } else {
            &sheet
                .rows
                .get(..)
                .map(|data| data.iter().map(|r| excel_helper::data_vec_to_string_vec(&r.iter().collect())).collect::<Vec<Vec<String>>>())
        }) else {
            continue;
        };

        let Some(first_row) = all_rows.get(0) else {
            report_sheet.warn(format!("未找到有效行: {reading_at}"));
            continue;
        };

        // println!("reading:  `{}`  rows: {}", reading_at, all_rows.len());

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

                let mut flat_key = flatten::FlattenKey::parse(key_raw.to_string(), |sub_key| {
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
                target_table_row.meta.push(((if is_transposed { row_i } else { col_j }) + 1).to_string());
            }
        }

        let mut final_output_path = if let Some(spec_output) = cfg.specified_output.get(&write_sheet_name) {
            path_helper::path_normalize(&base_path.join(spec_output))
        } else {
            path_helper::path_normalize(&base_path.join(&cfg.output).join(&write_sheet_name))
        };

        let content_kind = if let Some(st_ext) = final_output_path.extension() {
            ContentKinds::from(st_ext.to_str().unwrap_or(""))
        } else {
            final_output_path.set_extension(cfg_content_kind.as_str());
            cfg_content_kind
        };

        let body_result = match content_kind {
            ContentKinds::Json => get_json_list_by_table(&table_result),
            ContentKinds::Csv => get_table_list_by_table(&table_result, ","),
            _ => get_table_list_by_table(&table_result, "\t"),
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

fn filter_map_file<F>(entry: fs::DirEntry, predicate: F) -> Option<PathBuf>
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
    let path = path_clean::PathClean::clean(&base_dir.join(x));
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::File::create(&path)?;
    }
    Ok(Some(path))
}

pub fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = base_dir()?;

    let config_path = if Path::new(&cli.config).is_absolute() {
        PathBuf::from(&cli.config)
    } else {
        base_dir.join(&cli.config)
    };

    // 读取生成工具配置
    let config_content = if Path::new(&config_path).exists() {
        fs::read_to_string(&config_path)?
    } else {
        println!("> 📄 未找到配置: {:?}, 将使用默认配置", &config_path);
        String::new()
    };

    let mut cfg: Config = toml::from_str::<Config>(&config_content).expect("> ❌ 配置: 解析配置文件内容失败");

    // 命令行显式传入的参数优先级最高, 覆盖 toml 配置文件中读取到的同名配置项
    cfg.apply_cli_args(&cli.config_args);

    let enums_map: HashMap<String, HashMap<String, i64>> = cfg.enums.clone().into_iter().map(|(name, text)| (name, enum_helper::parse_enum_type(&text))).collect();

    let is_release = cfg.mode == "release";

    println!();
    println!("> 📄 读取配置: {:?}", &config_path);

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

    write_sheets(reports, &cfg)?;

    println!();
    println!("{}", format!("> 🕒 共用时: {:.6} 秒", execute_start.elapsed().as_secs_f64()).cyan());

    println!();
    if warn_count > 0 {
        println!("⚠️ 遇到 {} 个警告", warn_count);
        println!();
    }

    Ok(())
}
