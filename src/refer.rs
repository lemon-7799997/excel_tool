//! 引用模块: 将源 excel 某 sheet 区域的数据写入目标 excel 的对应位置。
//!
//! 完全独立的模块, 与现有生成业务无关。
//! 写入方式: 直接对目标 .xlsx(zip) 做最小修改 —— 只修改目标 sheet 的 XML 中
//! 目标矩形区域的单元格, 其余 zip 条目(公式/样式/数据验证/其它 sheet 等)字节级原封不动。

#![allow(dead_code)]
#![allow(unused_variables)]

use anyhow::{anyhow, bail, Context, Result};
use calamine::{open_workbook_auto, Data, Reader as CalamineReader};
use clap::Args;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader as XmlReader;
use quick_xml::writer::Writer;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::ZipArchive;

/// refer subcommand 参数
#[derive(Args, Debug)]
pub struct ReferArgs {
    /// 源 excel 文件路径
    pub from: PathBuf,

    /// 目标 excel 文件路径
    pub to: PathBuf,

    /// 源 sheet 名
    #[arg(long)]
    pub from_sheet: String,

    /// 目标 sheet 名
    #[arg(long)]
    pub to_sheet: String,

    /// 源列范围, 如 A:C 或 A (未给结束列时取该表已有数据最后一列)
    #[arg(long)]
    pub from_columns: Option<String>,

    /// 目标起始列, 如 A (只取起始列)
    #[arg(long)]
    pub to_columns: Option<String>,

    /// 源行范围, 如 2:10 或 2 (未给结束行时取该表已有数据最后一行)
    #[arg(long)]
    pub from_rows: Option<String>,

    /// 目标起始行, 如 2 (只取起始行)
    #[arg(long)]
    pub to_rows: Option<String>,

    /// 目标区域已有数据时直接覆盖, 不询问
    #[arg(short, long)]
    pub force: bool,
}

/// 单元格值
#[derive(Debug, Clone, PartialEq)]
enum CellValue {
    Str(String),
    Num(f64),
    Int(i64),
    Bool(bool),
    Err(String),
    Empty,
}

/// 源 sheet 数据
struct SheetData {
    /// 已有数据范围 (1-based 闭区间): (start_row, start_col, end_row, end_col)
    used: Option<(usize, usize, usize, usize)>,
    cells: HashMap<(usize, usize), CellValue>,
}

/// 入口
pub fn run(args: ReferArgs) -> Result<()> {
    // 2.3: 给了 from-参数但没有对应的 to-参数 -> 警告并跳过
    if args.from_columns.is_some() && args.to_columns.is_none() {
        println!("> ⚠️ 指定了 --from-columns 但没有 --to-columns, 跳过引用操作");
        return Ok(());
    }
    if args.from_rows.is_some() && args.to_rows.is_none() {
        println!("> ⚠️ 指定了 --from-rows 但没有 --to-rows, 跳过引用操作");
        return Ok(());
    }

    println!();
    println!(
        "> 📎 引用: {}#{} -> {}#{}",
        args.from.display(),
        args.from_sheet,
        args.to.display(),
        args.to_sheet
    );

    // 目标文件必须是 zip 结构的 xlsx/xlsm (zip 方案)
    let ext = args
        .to
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ext != "xlsx" && ext != "xlsm" {
        bail!("> ❌ 引用写入目前仅支持 .xlsx/.xlsm 目标文件, 当前: {}", args.to.display());
    }

    // 读取源 sheet
    let source = read_source_sheet(&args.from, &args.from_sheet)?;
    let Some((used_r0, used_c0, used_r1, used_c1)) = source.used else {
        println!("> ⚠️ 源 sheet `{}` 没有数据, 跳过引用操作", args.from_sheet);
        return Ok(());
    };

    // 解析范围参数
    let from_col_spec = match &args.from_columns {
        Some(s) => Some(parse_col_spec(s)?),
        None => None,
    };
    let from_row_spec = match &args.from_rows {
        Some(s) => Some(parse_row_spec(s)?),
        None => None,
    };

    // 2.2 / 2.4 / 2.5: 计算源矩形 (缺结束行/列时用已有数据的最后一行/列)
    let col_start = from_col_spec.as_ref().map(|s| s.start).unwrap_or(used_c0);
    let col_end = from_col_spec.as_ref().and_then(|s| s.end).unwrap_or(used_c1);
    let row_start = from_row_spec.as_ref().map(|s| s.start).unwrap_or(used_r0);
    let row_end = from_row_spec.as_ref().and_then(|s| s.end).unwrap_or(used_r1);

    let to_col = match &args.to_columns {
        Some(s) => parse_col_spec(s)?.start,
        None => 1,
    };
    let to_row = match &args.to_rows {
        Some(s) => parse_row_spec(s)?.start,
        None => 1,
    };

    let width = col_end.saturating_sub(col_start) + 1;
    let height = row_end.saturating_sub(row_start) + 1;

    println!(
        "> 📐 源区域: {}{}:{}{} ({}x{}), 写入目标起始: {}{}",
        col_letter(col_start),
        row_start,
        col_letter(col_end),
        row_end,
        width,
        height,
        col_letter(to_col),
        to_row
    );

    // 收集源矩形内所有值
    let mut values: Vec<Vec<CellValue>> = Vec::with_capacity(height);
    for r in row_start..=row_end {
        let mut row_vals = Vec::with_capacity(width);
        for c in col_start..=col_end {
            row_vals.push(source.cells.get(&(r, c)).cloned().unwrap_or(CellValue::Empty));
        }
        values.push(row_vals);
    }

    // 检查目标区域是否已有数据
    let mut target_wb = open_workbook_auto(&args.to)
        .with_context(|| format!("无法打开目标文件: {}", args.to.display()))?;
    let target_range = target_wb
        .worksheet_range(&args.to_sheet)
        .with_context(|| format!("目标文件中找不到 sheet: {}", args.to_sheet))?;

    let mut has_data = false;
    'outer: for r in to_row..to_row + height {
        for c in to_col..to_col + width {
            if let Some(v) = target_range.get_value(((r - 1) as u32, (c - 1) as u32)) {
                if !matches!(v, Data::Empty) {
                    has_data = true;
                    break 'outer;
                }
            }
        }
    }

    if has_data && !args.force {
        print!("> ⚠️ 目标区域已有数据, 输入 y 覆盖, 其它键取消: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        if line.trim().to_lowercase() != "y" {
            println!("> 已取消引用操作");
            return Ok(());
        }
    } else if has_data && args.force {
        println!("> ⚠️ 目标区域已有数据, 检测到 --force, 直接覆盖");
    }

    // 构建写入 map: (目标行, 目标列) -> 值
    let mut write_map: HashMap<(usize, usize), CellValue> = HashMap::new();
    for (ri, row_vals) in values.iter().enumerate() {
        for (ci, v) in row_vals.iter().enumerate() {
            write_map.insert((to_row + ri, to_col + ci), v.clone());
        }
    }
    let rect = (to_row, to_col, to_row + height - 1, to_col + width - 1);

    write_to_xlsx(&args.to, &args.to_sheet, &write_map, rect)?;

    println!(
        "> ✅ 引用完成: {} 行 x {} 列 已写入 {}#{} (起始 {}{})",
        height,
        width,
        args.to.display(),
        args.to_sheet,
        col_letter(to_col),
        to_row
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 读取源数据 (calamine)
// ---------------------------------------------------------------------------

fn read_source_sheet(path: &Path, sheet_name: &str) -> Result<SheetData> {
    let mut wb = open_workbook_auto(path)
        .with_context(|| format!("无法打开源文件: {}", path.display()))?;
    let range = wb
        .worksheet_range(sheet_name)
        .with_context(|| format!("源文件中找不到 sheet: {}", sheet_name))?;

    // calamine 内部坐标是 0-based, 转成 Excel 1-based
    let (sr, sc) = range.start().unwrap_or((0, 0));
    let (er, ec) = range.end().unwrap_or((0, 0));
    let (sr, sc) = (sr as usize + 1, sc as usize + 1);
    let (er, ec) = (er as usize + 1, ec as usize + 1);

    let mut cells = HashMap::new();
    let used = if er > 0 && ec > 0 && er >= sr && ec >= sc {
        for r in sr..=er {
            for c in sc..=ec {
                if let Some(v) = range.get_value(((r - 1) as u32, (c - 1) as u32)) {
                    if !matches!(v, Data::Empty) {
                        cells.insert((r, c), to_cell_value(v));
                    }
                }
            }
        }
        Some((sr, sc, er, ec))
    } else {
        None
    };

    Ok(SheetData { used, cells })
}

fn to_cell_value(d: &Data) -> CellValue {
    match d {
        Data::Empty => CellValue::Empty,
        Data::String(s) => CellValue::Str(s.clone()),
        Data::Float(f) => CellValue::Num(*f),
        Data::Int(i) => CellValue::Int(*i),
        Data::Bool(b) => CellValue::Bool(*b),
        Data::DateTime(f) => CellValue::Num(f.as_f64()),
        Data::DateTimeIso(s) => CellValue::Str(s.clone()),
        Data::DurationIso(s) => CellValue::Str(s.clone()),
        Data::Error(e) => CellValue::Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// 范围参数解析
// ---------------------------------------------------------------------------

struct ColSpec {
    start: usize,
    end: Option<usize>,
}

struct RowSpec {
    start: usize,
    end: Option<usize>,
}

fn parse_col_spec(s: &str) -> Result<ColSpec> {
    let parts: Vec<&str> = s.split(':').map(|p| p.trim()).collect();
    if parts.is_empty() || parts.len() > 2 || parts.iter().any(|p| p.is_empty()) {
        bail!("列范围格式不正确: {} (应为 A 或 A:C)", s);
    }
    let start = col_index(parts[0]).ok_or_else(|| anyhow!("无效列: {}", parts[0]))?;
    let end = if parts.len() == 2 {
        Some(col_index(parts[1]).ok_or_else(|| anyhow!("无效列: {}", parts[1]))?)
    } else {
        None
    };
    if let Some(e) = end {
        if e < start {
            bail!("列范围起点大于终点: {}", s);
        }
    }
    Ok(ColSpec { start, end })
}

fn parse_row_spec(s: &str) -> Result<RowSpec> {
    let parts: Vec<&str> = s.split(':').map(|p| p.trim()).collect();
    if parts.is_empty() || parts.len() > 2 || parts.iter().any(|p| p.is_empty()) {
        bail!("行范围格式不正确: {} (应为 2 或 2:10)", s);
    }
    let parse = |p: &str| -> Result<usize> { p.parse::<usize>().map_err(|_| anyhow!("无效行号: {}", p)) };
    let start = parse(parts[0])?;
    let end = if parts.len() == 2 { Some(parse(parts[1])?) } else { None };
    if let Some(e) = end {
        if e < start {
            bail!("行范围起点大于终点: {}", s);
        }
    }
    Ok(RowSpec { start, end })
}

/// 列字母 -> 1-based 列号: A->1, Z->26, AA->27
fn col_index(s: &str) -> Option<usize> {
    let s = s.trim().to_ascii_uppercase();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut v: u32 = 0;
    for ch in s.chars() {
        v = v * 26 + (ch as u32 - 'A' as u32 + 1);
    }
    Some(v as usize)
}

/// 1-based 列号 -> 列字母
fn col_letter(mut n: usize) -> String {
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, ((n % 26) as u8 + b'A') as char);
        n /= 26;
    }
    s
}

/// 解析单元格引用, 如 A1 或 A1:C5 -> (r0, c0, r1, c1) 1-based 闭区间
fn parse_cell_ref(s: &str) -> Result<(usize, usize, usize, usize)> {
    let s = s.trim();
    if s.is_empty() {
        bail!("空单元格引用");
    }
    let parts: Vec<&str> = s.split(':').collect();
    let (a, b) = match parts.as_slice() {
        [a] => (a, a),
        [a, b] => (a, b),
        _ => bail!("非法单元格引用: {}", s),
    };
    let (r1, c1) = parse_single_cell_ref(a)?;
    let (r2, c2) = parse_single_cell_ref(b)?;
    Ok((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
}

fn parse_single_cell_ref(s: &str) -> Result<(usize, usize)> {
    let s = s.trim();
    let split = s
        .find(|c: char| c.is_ascii_digit())
        .with_context(|| format!("非法单元格引用: {}", s))?;
    let (letters, digits) = s.split_at(split);
    let col = col_index(letters).with_context(|| format!("非法列字母: {}", letters))?;
    let row: usize = digits.parse().with_context(|| format!("非法行号: {}", digits))?;
    Ok((row, col))
}

// ---------------------------------------------------------------------------
// zip + XML 写入
// ---------------------------------------------------------------------------

fn write_to_xlsx(
    path: &Path,
    sheet_name: &str,
    write_map: &HashMap<(usize, usize), CellValue>,
    rect: (usize, usize, usize, usize),
) -> Result<()> {
    let file = File::open(path)
        .with_context(|| format!("无法打开目标文件: {}", path.display()))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("目标文件不是有效的 zip/xlsx: {}", path.display()))?;

    // 读取所有条目到内存
    let mut entries: Vec<(String, Vec<u8>, zip::CompressionMethod)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("读取 zip 条目失败: 索引 {}", i))?;
        let name = entry.name().to_string();
        let method = entry.compression();
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        entries.push((name, data, method));
    }
    drop(archive);

    // 定位目标 sheet 的 XML 条目
    let workbook_xml = entries
        .iter()
        .find(|(n, _, _)| n == "xl/workbook.xml")
        .with_context(|| "xlsx 缺少 xl/workbook.xml")?;
    let rels_xml = entries
        .iter()
        .find(|(n, _, _)| n == "xl/_rels/workbook.xml.rels")
        .with_context(|| "xlsx 缺少 xl/_rels/workbook.xml.rels")?;
    let sheet_part = find_sheet_part(&workbook_xml.1, &rels_xml.1, sheet_name)?;

    // 修改 sheet XML
    let sheet_data = entries
        .iter()
        .find(|(n, _, _)| n == &sheet_part)
        .with_context(|| format!("xlsx 缺少条目: {}", sheet_part))?;
    let new_sheet_xml = modify_sheet_xml(&sheet_data.1, write_map, rect)?;

    // 写回新 zip (先写临时文件再替换)
    let tmp_path = temp_path_for(path);
    let result = (|| -> Result<()> {
        let out = File::create(&tmp_path)
            .with_context(|| format!("无法创建临时文件: {}", tmp_path.display()))?;
        let mut writer = zip::ZipWriter::new(BufWriter::new(out));
        for (name, _data, method) in &entries {
            let data = if *name == sheet_part { &new_sheet_xml } else { _data };
            let options = SimpleFileOptions::default().compression_method(*method);
            writer
                .start_file(name, options)
                .with_context(|| format!("写 zip 条目失败: {}", name))?;
            writer.write_all(data)?;
        }
        writer.finish()?.into_inner()?;
        std::fs::rename(&tmp_path, path)
            .with_context(|| format!("替换目标文件失败: {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp.xlsx".to_string());
    path.with_file_name(format!(".{}.refer.tmp", file_name))
}

/// 通过 workbook.xml + workbook.xml.rels 找到 sheet 名对应的 sheet XML 条目路径
fn find_sheet_part(workbook_xml: &[u8], rels_xml: &[u8], sheet_name: &str) -> Result<String> {
    // 1) workbook.xml 中找 <sheet name=".." r:id=".."/>
    let mut rid: Option<String> = None;
    let mut reader = XmlReader::from_reader(Cursor::new(workbook_xml));
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"sheet" => {
                let mut name = None;
                let mut id = None;
                for attr in e.attributes().flatten() {
                    let key = attr.key.as_ref();
                    if key == b"name" {
                        name = Some(attr_str(&attr)?);
                    } else if key == b"r:id" || key == b"{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id" {
                        id = Some(attr_str(&attr)?);
                    }
                }
                if name.as_deref() == Some(sheet_name) {
                    rid = id;
                    break;
                }
            }
            Ok(_) => {}
            Err(e) => bail!("解析 workbook.xml 失败: {}", e),
        }
        buf.clear();
    }
    let rid = rid.with_context(|| format!("在 workbook.xml 中找不到 sheet: {}", sheet_name))?;

    // 2) rels 中找 <Relationship Id=".." Target=".."/>
    let mut target: Option<String> = None;
    let mut reader = XmlReader::from_reader(Cursor::new(rels_xml));
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"Relationship" => {
                let mut id = None;
                let mut t = None;
                for attr in e.attributes().flatten() {
                    let key = attr.key.as_ref();
                    if key == b"Id" {
                        id = Some(attr_str(&attr)?);
                    } else if key == b"Target" {
                        t = Some(attr_str(&attr)?);
                    }
                }
                if id.as_deref() == Some(rid.as_str()) {
                    target = t;
                    break;
                }
            }
            Ok(_) => {}
            Err(e) => bail!("解析 workbook.xml.rels 失败: {}", e),
        }
        buf.clear();
    }
    let target = target.with_context(|| format!("rels 中找不到 Id: {}", rid))?;
    let target = target.trim_start_matches('/');
    let part = if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{}", target)
    };
    Ok(part)
}

/// 读取并反转义属性值
fn attr_str(attr: &Attribute) -> Result<String> {
    let raw = std::str::from_utf8(attr.value.as_ref()).map_err(|_| anyhow!("属性值不是合法 UTF-8"))?;
    Ok(quick_xml::escape::unescape(raw)
        .map_err(|e| anyhow!("属性转义解析失败: {}", e))?
        .into_owned())
}

/// 修改 sheet XML: 只动目标矩形区域的单元格
fn modify_sheet_xml(
    xml: &[u8],
    write_map: &HashMap<(usize, usize), CellValue>,
    rect: (usize, usize, usize, usize),
) -> Result<Vec<u8>> {
    let (rect_r0, rect_c0, rect_r1, rect_c1) = rect;
    let target_cols: BTreeSet<usize> = (rect_c0..=rect_c1).collect();
    let target_rows: BTreeSet<usize> = (rect_r0..=rect_r1).collect();

    // 解析为 owned events
    let mut reader = XmlReader::from_reader(Cursor::new(xml));
    let mut buf = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(ev) => events.push(ev.into_owned()),
            Err(e) => bail!("解析 sheet XML 失败: {}", e),
        }
        buf.clear();
    }

    // 定位 dimension 与 sheetData
    let mut dimension_idx: Option<usize> = None;
    let mut sheet_data_start: Option<usize> = None;
    let mut sheet_data_end: Option<usize> = None;
    for (i, ev) in events.iter().enumerate() {
        match ev {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"dimension" => {
                if dimension_idx.is_none() {
                    dimension_idx = Some(i);
                }
            }
            Event::Start(e) if e.name().as_ref() == b"sheetData" => {
                if sheet_data_start.is_none() {
                    sheet_data_start = Some(i);
                }
            }
            Event::Empty(e) if e.name().as_ref() == b"sheetData" => {
                if sheet_data_start.is_none() {
                    sheet_data_start = Some(i);
                    sheet_data_end = Some(i);
                }
            }
            Event::End(e) if e.name().as_ref() == b"sheetData" => {
                if sheet_data_end.is_none() {
                    sheet_data_end = Some(i);
                }
            }
            _ => {}
        }
    }
    let s = sheet_data_start.context("sheet XML 缺少 sheetData")?;
    let e = sheet_data_end.context("sheet XML 缺少 </sheetData>")?;

    // 收集 sheetData 内部的行元素
    let mut row_slices: Vec<(usize, usize, usize)> = Vec::new(); // (start_idx, end_idx_inclusive, row_num)
    let mut other_idxs: Vec<usize> = Vec::new();
    let mut seq_row = 0usize;
    let mut i = s + 1;
    while i < e {
        match &events[i] {
            Event::Start(st) if st.name().as_ref() == b"row" => {
                let row_num = row_num_of(st, &mut seq_row)?;
                let mut depth = 1;
                let mut j = i + 1;
                while j < e && depth > 0 {
                    match &events[j] {
                        Event::Start(x) if x.name().as_ref() == b"row" => depth += 1,
                        Event::End(x) if x.name().as_ref() == b"row" => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                if depth != 0 {
                    bail!("row 元素未闭合");
                }
                row_slices.push((i, j - 1, row_num));
                i = j;
            }
            Event::Empty(st) if st.name().as_ref() == b"row" => {
                let row_num = row_num_of(st, &mut seq_row)?;
                row_slices.push((i, i, row_num));
                i += 1;
            }
            Event::Text(t) if t.iter().all(|b| b.is_ascii_whitespace()) => {
                i += 1;
            }
            _ => {
                other_idxs.push(i);
                i += 1;
            }
        }
    }

    // 处理已有行 + 新增行
    let mut existing_nums: BTreeSet<usize> = BTreeSet::new();
    let mut out_rows: Vec<(usize, Vec<Event>)> = Vec::new();
    for (si, ei, row_num) in &row_slices {
        existing_nums.insert(*row_num);
        if target_rows.contains(row_num) {
            let children: Vec<Event> = events[*si + 1..*ei].to_vec();
            let rebuilt = rebuild_row(*row_num, children, write_map, &target_cols);
            let was_empty = matches!(&events[*si], Event::Empty(_));
            out_rows.push((*row_num, make_row_element(*row_num, rebuilt, was_empty)));
        } else {
            out_rows.push((*row_num, events[*si..=*ei].to_vec()));
        }
    }
    for rn in target_rows.range(..) {
        if !existing_nums.contains(rn) {
            let cells = build_row_cells(*rn, &target_cols, write_map);
            if !cells.is_empty() {
                let mut row_evs = Vec::new();
                row_evs.push(Event::Start(row_start(*rn)));
                row_evs.extend(cells);
                row_evs.push(Event::End(BytesEnd::new("row")));
                out_rows.push((*rn, row_evs));
            }
        }
    }
    out_rows.sort_by_key(|(rn, _)| *rn);

    // 重建 sheetData 内容
    let mut new_sheet_data: Vec<Event> = Vec::new();
    for (_, evs) in &out_rows {
        new_sheet_data.extend(evs.iter().cloned());
    }
    for idx in &other_idxs {
        new_sheet_data.push(events[*idx].clone());
    }

    // 重建整体事件流
    let mut result: Vec<Event> = Vec::with_capacity(events.len() + 64);
    for (idx, ev) in events.iter().enumerate() {
        if idx == s {
            break;
        }
        if Some(idx) == dimension_idx {
            result.extend(update_dimension(ev, (rect_r0, rect_c0, rect_r1, rect_c1))?);
            continue;
        }
        result.push(ev.clone());
    }
    if e == s {
        // 原为 <sheetData/> 空元素
        result.push(Event::Start(BytesStart::new("sheetData")));
        result.extend(new_sheet_data);
        result.push(Event::End(BytesEnd::new("sheetData")));
    } else {
        result.push(events[s].clone());
        result.extend(new_sheet_data);
        result.push(events[e].clone());
    }
    for ev in events.iter().skip(e + 1) {
        result.push(ev.clone());
    }

    // 写出
    let mut writer = Writer::new(Vec::new());
    for ev in result {
        writer
            .write_event(ev)
            .map_err(|err| anyhow!("写出 sheet XML 失败: {}", err))?;
    }
    Ok(writer.into_inner())
}

/// 解析 <row> 元素的行号, 缺省时按顺序计数
fn row_num_of(st: &BytesStart<'_>, seq: &mut usize) -> Result<usize> {
    for attr in st.attributes().flatten() {
        if attr.key.as_ref() == b"r" {
            let v = attr_str(&attr)?;
            return v.parse::<usize>().map_err(|_| anyhow!("非法行号: {}", v));
        }
    }
    *seq += 1;
    Ok(*seq)
}

fn row_start(row_num: usize) -> BytesStart<'static> {
    let mut st = BytesStart::new("row");
    let r = row_num.to_string();
    push_attr(&mut st, "r", &r);
    st
}

fn make_row_element(row_num: usize, children: Vec<Event>, was_empty: bool) -> Vec<Event> {
    let mut evs = Vec::new();
    if children.is_empty() && was_empty {
        evs.push(Event::Empty(row_start(row_num)));
    } else {
        evs.push(Event::Start(row_start(row_num)));
        evs.extend(children);
        evs.push(Event::End(BytesEnd::new("row")));
    }
    evs
}

/// 重建一行: 覆盖目标列单元格, 保留其它内容
fn rebuild_row(
    row_num: usize,
    children: Vec<Event<'static>>,
    write_map: &HashMap<(usize, usize), CellValue>,
    target_cols: &BTreeSet<usize>,
) -> Vec<Event<'static>> {
    // 1) 找出所有 <c> 元素的事件范围
    let mut cell_ranges: Vec<(usize, usize, usize, Option<String>)> = Vec::new(); // (start, end_inclusive, col, s_attr)
    let mut seq_col = 0usize;
    let mut i = 0;
    while i < children.len() {
        let is_cell_start = match &children[i] {
            Event::Start(st) | Event::Empty(st) => st.name().as_ref() == b"c",
            _ => false,
        };
        if is_cell_start {
            let (col, s_attr) = cell_col_of(&children[i], &mut seq_col);
            if matches!(&children[i], Event::Start(_)) {
                let mut depth = 1;
                let mut j = i + 1;
                while j < children.len() && depth > 0 {
                    match &children[j] {
                        Event::Start(x) if x.name().as_ref() == b"c" => depth += 1,
                        Event::End(x) if x.name().as_ref() == b"c" => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                if depth != 0 {
                    break;
                }
                cell_ranges.push((i, j - 1, col, s_attr));
                i = j;
            } else {
                cell_ranges.push((i, i, col, s_attr));
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    // 2) 已有列 -> 样式 s
    let mut existing: BTreeMap<usize, Option<String>> = BTreeMap::new();
    for (_, _, col, s_attr) in &cell_ranges {
        existing.insert(*col, s_attr.clone());
    }

    // 3) 目标列的单元格事件 (覆盖已有 / 新建)
    let mut pending: BTreeMap<usize, Vec<Event>> = BTreeMap::new();
    for col in target_cols {
        let value = write_map.get(&(row_num, *col)).unwrap_or(&CellValue::Empty);
        let cell_ref = format!("{}{}", col_letter(*col), row_num);
        if let Some(s) = existing.get(col) {
            pending.insert(*col, build_cell_events(&cell_ref, s.as_deref(), value));
        } else if !matches!(value, CellValue::Empty) {
            pending.insert(*col, build_cell_events(&cell_ref, None, value));
        }
    }

    // 4) 组装: 非单元格内容原样保留, 单元格按列排序
    let mut out: Vec<Event> = Vec::new();
    let mut pend_iter = pending.into_iter().peekable();
    let mut cursor = 0usize;
    for (cs, ce, col, _) in &cell_ranges {
        for ev in &children[cursor..*cs] {
            out.push(ev.clone());
        }
        while let Some((pc, pvs)) = pend_iter.peek() {
            if *pc < *col {
                out.extend(pvs.iter().cloned());
                pend_iter.next();
            } else {
                break;
            }
        }
        if target_cols.contains(col) {
            if let Some((pc, pvs)) = pend_iter.peek() {
                if *pc == *col {
                    out.extend(pvs.iter().cloned());
                    pend_iter.next();
                }
            }
        } else {
            out.extend(children[*cs..=*ce].iter().cloned());
        }
        cursor = *ce + 1;
    }
    for ev in &children[cursor..] {
        out.push(ev.clone());
    }
    for (_, pvs) in pend_iter {
        out.extend(pvs.iter().cloned());
    }
    out
}

/// 解析 <c> 单元格的列号, 缺 r 属性时按顺序计数
fn cell_col_of(ev: &Event<'_>, seq: &mut usize) -> (usize, Option<String>) {
    let st = match ev {
        Event::Start(s) | Event::Empty(s) => s,
        _ => {
            *seq += 1;
            return (*seq, None);
        }
    };
    let mut s_attr = None;
    let mut col = None;
    for attr in st.attributes().flatten() {
        let key = attr.key.as_ref();
        if key == b"r" {
            if let Ok(v) = attr_str(&attr) {
                if let Ok((_, c)) = parse_single_cell_ref(&v) {
                    col = Some(c);
                }
            }
        } else if key == b"s" {
            s_attr = Some(attr_str(&attr).unwrap_or_default());
        }
    }
    match col {
        Some(c) => (c, s_attr),
        None => {
            *seq += 1;
            (*seq, s_attr)
        }
    }
}

/// 生成一个单元格的完整事件序列
fn build_cell_events(cell_ref: &str, s_attr: Option<&str>, value: &CellValue) -> Vec<Event<'static>> {
    let mut out = Vec::new();
    match value {
        CellValue::Empty => {
            let mut st = BytesStart::new("c");
            push_attr(&mut st, "r", cell_ref);
            if let Some(s) = s_attr {
                push_attr(&mut st, "s", s);
            }
            out.push(Event::Empty(st));
        }
        CellValue::Str(text) => {
            let mut st = BytesStart::new("c");
            push_attr(&mut st, "r", cell_ref);
            if let Some(s) = s_attr {
                push_attr(&mut st, "s", s);
            }
            push_attr(&mut st, "t", "inlineStr");
            out.push(Event::Start(st));
            out.push(Event::Start(BytesStart::new("is")));
            out.push(Event::Start(BytesStart::new("t")));
            out.push(Event::Text(BytesText::new(text).into_owned()));
            out.push(Event::End(BytesEnd::new("t")));
            out.push(Event::End(BytesEnd::new("is")));
            out.push(Event::End(BytesEnd::new("c")));
        }
        CellValue::Num(n) => {
            let text = format!("{}", n);
            let mut st = BytesStart::new("c");
            push_attr(&mut st, "r", cell_ref);
            if let Some(s) = s_attr {
                push_attr(&mut st, "s", s);
            }
            out.push(Event::Start(st));
            out.push(Event::Start(BytesStart::new("v")));
            out.push(Event::Text(BytesText::new(&text).into_owned()));
            out.push(Event::End(BytesEnd::new("v")));
            out.push(Event::End(BytesEnd::new("c")));
        }
        CellValue::Int(n) => {
            let text = format!("{}", n);
            let mut st = BytesStart::new("c");
            push_attr(&mut st, "r", cell_ref);
            if let Some(s) = s_attr {
                push_attr(&mut st, "s", s);
            }
            out.push(Event::Start(st));
            out.push(Event::Start(BytesStart::new("v")));
            out.push(Event::Text(BytesText::new(&text).into_owned()));
            out.push(Event::End(BytesEnd::new("v")));
            out.push(Event::End(BytesEnd::new("c")));
        }
        CellValue::Bool(b) => {
            let text = if *b { "1" } else { "0" };
            let mut st = BytesStart::new("c");
            push_attr(&mut st, "r", cell_ref);
            if let Some(s) = s_attr {
                push_attr(&mut st, "s", s);
            }
            push_attr(&mut st, "t", "b");
            out.push(Event::Start(st));
            out.push(Event::Start(BytesStart::new("v")));
            out.push(Event::Text(BytesText::new(text).into_owned()));
            out.push(Event::End(BytesEnd::new("v")));
            out.push(Event::End(BytesEnd::new("c")));
        }
        CellValue::Err(e) => {
            let mut st = BytesStart::new("c");
            push_attr(&mut st, "r", cell_ref);
            if let Some(s) = s_attr {
                push_attr(&mut st, "s", s);
            }
            push_attr(&mut st, "t", "e");
            out.push(Event::Start(st));
            out.push(Event::Start(BytesStart::new("v")));
            out.push(Event::Text(BytesText::new(e).into_owned()));
            out.push(Event::End(BytesEnd::new("v")));
            out.push(Event::End(BytesEnd::new("c")));
        }
    }
    out
}

fn push_attr(st: &mut BytesStart<'static>, key: &str, value: &str) {
    st.push_attribute((key, value));
}

/// 新增行: 为目标列生成单元格事件 (空值跳过)
fn build_row_cells(
    row_num: usize,
    cols: &BTreeSet<usize>,
    write_map: &HashMap<(usize, usize), CellValue>,
) -> Vec<Event<'static>> {
    let mut out = Vec::new();
    for col in cols {
        if let Some(v) = write_map.get(&(row_num, *col)) {
            if matches!(v, CellValue::Empty) {
                continue;
            }
            let cell_ref = format!("{}{}", col_letter(*col), row_num);
            out.extend(build_cell_events(&cell_ref, None, v));
        }
    }
    out
}

/// 更新 <dimension> 的 ref, 使其包含写入的矩形
fn update_dimension(ev: &Event<'_>, rect: (usize, usize, usize, usize)) -> Result<Vec<Event<'static>>> {
    let (r0, c0, r1, c1) = rect;
    let st = match ev {
        Event::Start(s) | Event::Empty(s) => s,
        _ => return Ok(vec![ev.clone().into_owned()]),
    };
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut new_ref: Option<String> = None;
    for attr in st.attributes().flatten() {
        let key = attr.key.as_ref();
        let value = attr_str(&attr)?;
        if key == b"ref" {
            let (dr0, dc0, dr1, dc1) = parse_cell_ref(&value).unwrap_or((r0, c0, r1, c1));
            let (nr0, nc0) = (dr0.min(r0), dc0.min(c0));
            let (nr1, nc1) = (dr1.max(r1), dc1.max(c1));
            new_ref = Some(format!(
                "{}{}:{}{}",
                col_letter(nc0),
                nr0,
                col_letter(nc1),
                nr1
            ));
        } else {
            attrs.push((String::from_utf8_lossy(key).into_owned(), value));
        }
    }
    if let Some(rf) = new_ref {
        attrs.push(("ref".to_string(), rf));
        let mut start = BytesStart::new("dimension");
        for (k, v) in &attrs {
            push_attr(&mut start, k, v);
        }
        return Ok(vec![Event::Empty(start)]);
    }
    Ok(vec![ev.clone().into_owned()])
}
