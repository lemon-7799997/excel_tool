pub fn filter_table<RowFilter, ColFilter>(rows: Vec<Vec<String>>, row_filter: RowFilter, col_filter: ColFilter) -> Vec<Vec<String>>
where
    RowFilter: Fn(&Vec<String>) -> bool,
    ColFilter: Fn(usize, &String) -> bool,
{
    if rows.is_empty() {
        return Vec::new();
    }

    let header = &rows[0];

    // 计算保留哪些列
    let mut keep_cols = Vec::with_capacity(header.len());

    for (i, cell) in header.iter().enumerate() {
        keep_cols.push(col_filter(i, cell));
    }

    let mut result = Vec::new();

    for row in rows {
        if !row_filter(&row) {
            continue;
        }

        let mut new_row = Vec::new();

        for (i, cell) in row.into_iter().enumerate() {
            if keep_cols.get(i).copied().unwrap_or(true) {
                new_row.push(cell);
            }
        }

        result.push(new_row);
    }

    result
}

pub fn print_table(table: &Vec<Vec<String>>) {
    for (i, row) in table.iter().enumerate() {
        println!("[{i}] {}", row.join("\t"));
    }
}

