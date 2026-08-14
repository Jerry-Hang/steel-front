//! TOML 关卡地图解析 + MapManager（手写轻量 TOML 子集解析器，零第三方依赖）。
//!
//! 支持的语法子集（值一律单行，与内联表/内联表数组的 TOML 规则一致）：
//! - `[map]` 节 + `name = "..."` / `description = "..."` 字符串键值
//! - `spawn_points = [ { x = .., y = .., z = .., team = "blue" }, ... ]` 内联表数组
//! - `objectives = [ { id = "..", type = "capture", position = { x,y,z }, radius = .., capture_time = .. }, ... ]`
//! - `obstacles = [ { type = "wall", position = { x,y,z }, size = { x,y,z } }, ... ]`
//! - 键值对 / 内联表 `{ ... }` / 内联表数组 / 整数与浮点（含负数、小数、科学计数）/ UTF-8 字符串（含中文）
//! - 行注释 `# ...`（字符串字面量内的 `#` 保留）
//!
//! 不支持完整 TOML 规范（`[[array-of-tables]]`、日期、布尔数组、多行值等）；遇到未知键/未知节静默忽略。
//! 解析错误返回带行号的 `Result<_, String>`。
//!
//! TOML 障碍是 `position{x,y,z}` + `size{x,y,z}`；映射到物理侧 `MapObstacle`（盒中心 + x/z 半尺寸）
//! 时用 `position.x/z` + `MAP_BLOCK_HEIGHT` 高度，见 `obstacle_to_map_obstacle`。

#![allow(dead_code)] // 主会话接线（main.rs/game.rs）前公开 API 暂未被非测试代码引用；测试已全量覆盖，接线后可移除

use crate::engine::game::ObstacleKind;
use std::cell::Cell;

/// 障碍贴地高度（米）：TOML 障碍只给 `position.y`，物理侧统一用该常量（与 game.rs 同值）。
pub const MAP_BLOCK_HEIGHT: f32 = 2.4;

// ============================================================
// 数据结构
// ============================================================

/// 关卡出生点（`team`: "blue" / "red"，匹配忽略大小写；缺失 team 视为双方可用）
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub team: String,
}

/// TOML 障碍定义（`kind`: "wall" / "block" / "barrier" / "cover"）
#[derive(Debug, Clone, PartialEq)]
pub struct ObstacleDef {
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub sx: f32,
    pub sy: f32,
    pub sz: f32,
}

/// TOML 任务目标（`kind`: "capture" 等；TOML 键 `type` 或 `kind` 均可）
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveDef {
    pub id: String,
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub radius: f32,
    pub capture_time: f32,
}

/// 解析后的完整关卡数据
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapData {
    pub name: String,
    pub description: String,
    pub spawn_points: Vec<SpawnPoint>,
    pub objectives: Vec<ObjectiveDef>,
    pub obstacles: Vec<ObstacleDef>,
    /// 胜负规则（[rule] 节；缺失时默认 CapturePoints{required=1}）
    pub rule: RuleDef,
}

/// 胜负规则（TOML `[rule]` 节）：
/// - `kind`: "capture"（占领 required 个据点）/ "kill"（击杀 target 名）/ "time"（限时 seconds 秒）
/// - 与 objective.rs `GameRule` 一一对应；未知 kind 由主会话回退默认
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleDef {
    pub kind: String,
    pub required: usize,
    pub target: u32,
    pub seconds: f64,
}

// ============================================================
// 轻量 TOML 值模型
// ============================================================

/// 解析出的 TOML 值（本 schema 子集）
#[derive(Debug, Clone)]
enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    Table(Vec<(String, Value)>),
    Array(Vec<Value>),
}

impl Value {
    /// 若为内联表，返回键值对切片
    fn as_table(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// 带行号的解析错误
fn err(ln: usize, msg: impl Into<String>) -> String {
    format!("map: 第 {} 行: {}", ln, msg.into())
}

/// 字符串内未闭合 `[`/`{` 的净深度（忽略字符串字面量内的括号与转义）。
/// >0 表示还有未闭合的左括号，需要继续吞并后续行。
fn bracket_depth(s: &str) -> i32 {
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut prev_esc = false;
    for c in s.chars() {
        if in_str {
            if prev_esc {
                prev_esc = false;
            } else if c == '\\' {
                prev_esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// 键存在则覆盖，否则追加（重复键后者生效）
fn insert_or_replace(entries: &mut Vec<(String, Value)>, key: String, value: Value) {
    if let Some(e) = entries.iter_mut().find(|(k, _)| *k == key) {
        e.1 = value;
    } else {
        entries.push((key, value));
    }
}

/// 去掉行注释（`#` 至行尾；字符串字面量内的 `#` 保留）
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut prev_esc = false;
    for (i, c) in line.char_indices() {
        if in_str {
            if prev_esc {
                prev_esc = false;
            } else if c == '\\' {
                prev_esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '#' {
            return &line[..i];
        }
    }
    line
}

/// 在引号之外查找第一个 `=`（键值分隔符）
fn find_eq(line: &str) -> Option<usize> {
    let mut in_str = false;
    let mut prev_esc = false;
    for (i, c) in line.char_indices() {
        if in_str {
            if prev_esc {
                prev_esc = false;
            } else if c == '\\' {
                prev_esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '=' {
            return Some(i);
        }
    }
    None
}

/// 合法裸键：字母/数字/下划线/连字符
fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 在顶层（引号/括号深度 0）按分隔符拆分；`{ }` 与 `[ ]` 内的分隔符不计入
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut prev_esc = false;
    for (i, c) in s.char_indices() {
        if in_str {
            if prev_esc {
                prev_esc = false;
            } else if c == '\\' {
                prev_esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ if c == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

// ============================================================
// TOML 解析器（行扫描 + 单行内联结构）
// ============================================================

struct TomlParser {
    /// 预处理后的行（原始行号, 去注释内容）
    lines: Vec<(usize, String)>,
    idx: usize,
}

impl TomlParser {
    fn new(text: &str) -> Self {
        // 去掉 UTF-8 BOM（编辑器可能带）
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let lines = text
            .lines()
            .enumerate()
            .map(|(i, l)| (i + 1, strip_comment(l).to_string()))
            .collect();
        Self { lines, idx: 0 }
    }

    /// 解析全文 → MapData；未知节/未知键静默忽略
    fn parse(&mut self) -> Result<MapData, String> {
        let mut entries: Vec<(String, Value)> = Vec::new();
        let mut rule_entries: Vec<(String, Value)> = Vec::new();
        let mut section = String::new(); // 当前节名（空 = 未进入任何节）
        while self.idx < self.lines.len() {
            // 克隆行（含行号）再前进，避免与 parse_kv 的 &mut self 冲突
            let (ln, raw) = self.lines[self.idx].clone();
            let line = raw.trim();
            self.idx += 1;
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                section = self.parse_section_header(ln, line)?;
                continue;
            }
            match section.as_str() {
                "map" => {
                    let (key, value) = self.parse_kv_multiline(ln, line)?;
                    insert_or_replace(&mut entries, key, value);
                }
                "rule" => {
                    let (key, value) = self.parse_kv_multiline(ln, line)?;
                    insert_or_replace(&mut rule_entries, key, value);
                }
                _ => continue, // 其它节静默忽略
            }
        }
        if entries.is_empty() {
            return Err(err(
                1,
                format!("未找到 [map] 节（文件共 {} 行）", self.lines.len()),
            ));
        }
        Ok(assemble_map(entries, rule_entries))
    }

    fn parse_section_header(&self, ln: usize, line: &str) -> Result<String, String> {
        if !line.ends_with(']') || line.starts_with("[[") {
            return Err(err(
                ln,
                "无效的节头（本子集不支持 [[array-of-tables]]，节头需以 ] 收尾）",
            ));
        }
        Ok(line[1..line.len() - 1].trim().to_string())
    }

    fn parse_kv(&mut self, ln: usize, line: &str) -> Result<(String, Value), String> {
        let eq = find_eq(line).ok_or_else(|| err(ln, "键值对缺少 '='"))?;
        let key = line[..eq].trim();
        if !is_valid_key(key) {
            return Err(err(ln, format!("无效键名 \"{}\"", key)));
        }
        let value = self.parse_value(ln, line[eq + 1..].trim())?;
        Ok((key.to_string(), value))
    }

    /// 解析键值对，支持跨行值：值含未闭合 `[`/`{` 时继续吞并后续行直到括号闭合。
    /// 多行内容用空格拼接（对内联数组/表而言换行即空白，语义不变）。
    fn parse_kv_multiline(&mut self, ln: usize, line: &str) -> Result<(String, Value), String> {
        let eq = find_eq(line).ok_or_else(|| err(ln, "键值对缺少 '='"))?;
        let key = line[..eq].trim();
        if !is_valid_key(key) {
            return Err(err(ln, format!("无效键名 \"{}\"", key)));
        }
        let mut value_str = line[eq + 1..].trim().to_string();
        // 括号深度检查（忽略字符串字面量内的括号）
        while bracket_depth(&value_str) > 0 && self.idx < self.lines.len() {
            let (_, raw) = self.lines[self.idx].clone();
            self.idx += 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            // 拼接行前先检查是否进入了新节（多行数组不应包含 [xxx] 节头）
            if trimmed.starts_with('[') && bracket_depth(&value_str) == 0 {
                break;
            }
            value_str.push(' ');
            value_str.push_str(trimmed);
        }
        let value = self.parse_value(ln, &value_str)?;
        Ok((key.to_string(), value))
    }

    fn parse_value(&mut self, ln: usize, s: &str) -> Result<Value, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err(err(ln, "缺少值"));
        }
        let (value, consumed) = match s.as_bytes()[0] {
            b'"' => {
                let (v, c) = self.parse_string(ln, s)?;
                (Value::Str(v), c)
            }
            b'{' => {
                let (v, c) = self.parse_inline_table(ln, s)?;
                (Value::Table(v), c)
            }
            b'[' => {
                let (v, c) = self.parse_inline_array(ln, s)?;
                (Value::Array(v), c)
            }
            _ => {
                match s {
                    "true" => (Value::Bool(true), s.len()),
                    "false" => (Value::Bool(false), s.len()),
                    _ => (Value::Num(self.parse_number(ln, s)?), s.len()),
                }
            }
        };
        let rest = s[consumed..].trim();
        if !rest.is_empty() {
            return Err(err(ln, format!("值后存在多余内容 \"{}\"", rest)));
        }
        Ok(value)
    }

    /// 解析双引号字符串（含 `\\` `\"` `\n` `\t` `\r` 转义；未知转义保留字面字符）。返回 (值, 消耗字节数)
    fn parse_string(&self, ln: usize, s: &str) -> Result<(String, usize), String> {
        let mut chars = s.char_indices();
        chars.next(); // 跳过开引号
        let mut out = String::new();
        while let Some((i, c)) = chars.next() {
            if c == '"' {
                return Ok((out, i + c.len_utf8()));
            }
            if c == '\\' {
                match chars.next() {
                    Some((_, '"')) => out.push('"'),
                    Some((_, '\\')) => out.push('\\'),
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, 'r')) => out.push('\r'),
                    Some((_, other)) => out.push(other), // 宽松：未知转义保留字面字符
                    None => return Err(err(ln, "字符串转义不完整")),
                }
            } else {
                out.push(c);
            }
        }
        Err(err(ln, "字符串未闭合（缺少结尾引号）"))
    }

    fn parse_number(&self, ln: usize, s: &str) -> Result<f64, String> {
        s.parse::<f64>()
            .map_err(|_| err(ln, format!("无效数字 \"{}\"", s)))
    }

    /// 从 `s` 起始处扫描配对定界符包裹的内容，返回 (内容切片, 消耗字节数)
    fn scan_braced<'a>(
        &self,
        ln: usize,
        s: &'a str,
        open: char,
        close: char,
    ) -> Result<(&'a str, usize), String> {
        let mut depth: i32 = 0;
        let mut in_str = false;
        let mut prev_esc = false;
        for (i, c) in s.char_indices() {
            if in_str {
                if prev_esc {
                    prev_esc = false;
                } else if c == '\\' {
                    prev_esc = true;
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                _ if c == open => depth += 1,
                _ if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok((&s[1..i], i + c.len_utf8()));
                    }
                }
                _ => {}
            }
        }
        Err(err(ln, format!("缺少 '{}' 结束符", close)))
    }

    /// 解析内联表 `{ key = value, ... }`，返回 (条目, 消耗字节数)
    fn parse_inline_table(
        &mut self,
        ln: usize,
        s: &str,
    ) -> Result<(Vec<(String, Value)>, usize), String> {
        let (content, consumed) = self.scan_braced(ln, s, '{', '}')?;
        let mut entries = Vec::new();
        for part in split_top(content, ',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let eq = find_eq(part).ok_or_else(|| err(ln, "内联表条目缺少 '='"))?;
            let key = part[..eq].trim();
            if !is_valid_key(key) {
                return Err(err(ln, format!("无效键名 \"{}\"", key)));
            }
            let value = self.parse_value(ln, part[eq + 1..].trim())?;
            insert_or_replace(&mut entries, key.to_string(), value);
        }
        Ok((entries, consumed))
    }

    /// 解析内联数组 `[ elem, ... ]`，返回 (元素, 消耗字节数)
    fn parse_inline_array(&mut self, ln: usize, s: &str) -> Result<(Vec<Value>, usize), String> {
        let (content, consumed) = self.scan_braced(ln, s, '[', ']')?;
        let mut items = Vec::new();
        for part in split_top(content, ',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            items.push(self.parse_value(ln, part)?);
        }
        Ok((items, consumed))
    }
}

// ============================================================
// 提取辅助（缺键返回默认值；未知键天然忽略）
// ============================================================

fn table_get<'a>(entries: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn table_str(entries: &[(String, Value)], key: &str) -> String {
    match table_get(entries, key) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn table_f32(entries: &[(String, Value)], key: &str) -> f32 {
    match table_get(entries, key) {
        Some(Value::Num(n)) => *n as f32,
        _ => 0.0,
    }
}

/// 取三维子表 `{ x, y, z }`（缺子表或缺分量 → 0.0）
fn table_vec3(value: &Value) -> (f32, f32, f32) {
    match value {
        Value::Table(entries) => (
            table_f32(entries, "x"),
            table_f32(entries, "y"),
            table_f32(entries, "z"),
        ),
        _ => (0.0, 0.0, 0.0),
    }
}

fn table_vec3_value(entries: &[(String, Value)], key: &str) -> (f32, f32, f32) {
    match table_get(entries, key) {
        Some(v) => table_vec3(v),
        None => (0.0, 0.0, 0.0),
    }
}

/// 取键对应的数组元素表列表；缺键/非数组 → 空
fn table_array<'a>(entries: &'a [(String, Value)], key: &str) -> Vec<&'a [(String, Value)]> {
    match table_get(entries, key) {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_table).collect(),
        _ => Vec::new(),
    }
}

fn parse_spawn_point(t: &[(String, Value)]) -> SpawnPoint {
    SpawnPoint {
        x: table_f32(t, "x"),
        y: table_f32(t, "y"),
        z: table_f32(t, "z"),
        team: table_str(t, "team"),
    }
}

fn parse_objective(t: &[(String, Value)]) -> ObjectiveDef {
    let (x, y, z) = table_vec3_value(t, "position");
    // TOML 键 `type`（schema 约定）；缺省回退 `kind` 同义键
    let kind = table_str(t, "type");
    let kind = if kind.is_empty() { table_str(t, "kind") } else { kind };
    ObjectiveDef {
        id: table_str(t, "id"),
        kind,
        x,
        y,
        z,
        radius: table_f32(t, "radius"),
        capture_time: table_f32(t, "capture_time"),
    }
}

fn parse_obstacle(t: &[(String, Value)]) -> ObstacleDef {
    let (x, y, z) = table_vec3_value(t, "position");
    let (sx, sy, sz) = table_vec3_value(t, "size");
    ObstacleDef {
        kind: table_str(t, "type"),
        x,
        y,
        z,
        sx,
        sy,
        sz,
    }
}

/// 把 `[map]` 节的键值组装为 MapData；`[rule]` 节键值组装为胜负规则（缺失 → 默认 capture/required=1）
fn assemble_map(entries: Vec<(String, Value)>, rule_entries: Vec<(String, Value)>) -> MapData {
    let rule = RuleDef {
        kind: rule_str(&rule_entries, "kind")
            .unwrap_or_else(|| "capture".to_string())
            .to_lowercase(),
        // required 默认 0：capture 语义（至少 1）由主会话组装 GameRule 时 max(1) 兜底
        required: rule_usize(&rule_entries, "required").unwrap_or(0),
        target: rule_usize(&rule_entries, "target").unwrap_or(0) as u32,
        seconds: rule_f64(&rule_entries, "seconds").unwrap_or(0.0),
    };
    MapData {
        name: table_str(&entries, "name"),
        description: table_str(&entries, "description"),
        spawn_points: table_array(&entries, "spawn_points")
            .into_iter()
            .map(parse_spawn_point)
            .collect(),
        objectives: table_array(&entries, "objectives")
            .into_iter()
            .map(parse_objective)
            .collect(),
        obstacles: table_array(&entries, "obstacles")
            .into_iter()
            .map(parse_obstacle)
            .collect(),
        rule,
    }
}

/// [rule] 节字符串值（缺失 → None）
fn rule_str(entries: &[(String, Value)], key: &str) -> Option<String> {
    entries.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    })
}

/// [rule] 节数值（缺失 → None）
fn rule_usize(entries: &[(String, Value)], key: &str) -> Option<usize> {
    entries.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
        Value::Num(n) => Some(*n as usize),
        _ => None,
    })
}

/// [rule] 节浮点值（缺失 → None）
fn rule_f64(entries: &[(String, Value)], key: &str) -> Option<f64> {
    entries.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
        Value::Num(n) => Some(*n),
        _ => None,
    })
}

// ============================================================
// 公开 API
// ============================================================

/// 解析 TOML 子集文本为 MapData
fn parse_map_toml(text: &str) -> Result<MapData, String> {
    TomlParser::new(text).parse()
}

/// 读取并解析关卡地图文件，并做基本校验（≥1 出生点；≥1 障碍或目标）
pub fn load_map(path: &str) -> Result<MapData, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("map: 无法读取 {}: {}", path, e))?;
    let data = parse_map_toml(&text)?;
    if data.spawn_points.is_empty() {
        return Err(format!(
            "map: {} 至少需要 1 个 spawn_point（当前 0 个）",
            path
        ));
    }
    if data.obstacles.is_empty() && data.objectives.is_empty() {
        return Err(format!(
            "map: {} 至少需要 1 个 obstacle 或 1 个 objective（当前全空）",
            path
        ));
    }
    Ok(data)
}

/// 读取关卡列表（index.toml）：`maps = ["a.toml", "b.toml"]`。
/// 返回按列表顺序的关卡文件路径（相对 index.toml 所在目录拼接）。
/// 至少 1 个条目，否则 Err；条目不做存在性校验（由后续 load_map 报错）。
pub fn load_map_list(index_path: &str) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(index_path)
        .map_err(|e| format!("map: 无法读取关卡列表 {}: {}", index_path, e))?;
    let data = parse_map_list_toml(&text)?;
    if data.is_empty() {
        return Err(format!(
            "map: 关卡列表 {} 中没有 maps 条目（需至少 1 个）",
            index_path
        ));
    }
    // 相对路径按 index.toml 目录拼接（绝对路径原样返回）
    let dir = std::path::Path::new(index_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    Ok(data
        .into_iter()
        .map(|p| {
            let pp = std::path::Path::new(&p);
            if pp.is_absolute() {
                p
            } else {
                dir.join(pp).to_string_lossy().into_owned()
            }
        })
        .collect())
}

/// 解析关卡列表 TOML 子集：仅 `maps = ["...", "..."]` 字符串数组
fn parse_map_list_toml(text: &str) -> Result<Vec<String>, String> {
    let mut parser = TomlParser::new(text);
    let mut names: Vec<String> = Vec::new();
    while parser.idx < parser.lines.len() {
        let (ln, raw) = parser.lines[parser.idx].clone();
        let line = raw.trim();
        parser.idx += 1;
        if line.is_empty() || line.starts_with('[') || line.starts_with('#') {
            continue;
        }
        let eq = find_eq(line).ok_or_else(|| err(ln, "键值对缺少 '='"))?;
        let key = line[..eq].trim();
        if key != "maps" {
            continue; // 未知键忽略
        }
        let (_, value) = parser.parse_kv_multiline(ln, line)?;
        match value {
            Value::Array(items) => {
                for item in items {
                    if let Value::Str(s) = item {
                        names.push(s);
                    }
                }
            }
            _ => return Err(err(ln, "maps 必须是字符串数组")),
        }
    }
    Ok(names)
}

/// 关卡地图管理器（热重载入口：F5 → `reload`）
pub struct MapManager {
    current: MapData,
    /// LCG 状态（Cell 以便 `&self` 内推进；种子由地图名派生，同图恒同序列）
    seed: Cell<u32>,
}

/// 确定性种子：地图名的 FNV-1a 哈希（同图恒同出生点序列）
fn seed_from_name(name: &str) -> u32 {
    let mut h = 0x811C_9DC5u32;
    for b in name.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl MapManager {
    /// 空管理器（未加载任何地图）
    pub fn new() -> Self {
        Self {
            current: MapData::default(),
            seed: Cell::new(seed_from_name("")),
        }
    }

    /// 加载地图文件（失败返回带原因的 Err）
    pub fn load(path: &str) -> Result<Self, String> {
        let data = load_map(path)?;
        let seed = seed_from_name(&data.name);
        Ok(Self {
            current: data,
            seed: Cell::new(seed),
        })
    }

    /// 热重载：重新读取并替换当前地图（F5 调用）
    pub fn reload(&mut self, path: &str) -> Result<(), String> {
        let data = load_map(path)?;
        self.seed.set(seed_from_name(&data.name));
        self.current = data;
        Ok(())
    }

    /// 当前地图数据
    pub fn data(&self) -> &MapData {
        &self.current
    }

    /// 该阵营出生点（team 匹配忽略大小写；缺失 team 的出生点双方可用）。
    /// 从候选出生点中按确定性 LCG 随机选一个。
    pub fn spawn_point(&self, team: &str) -> Option<(f32, f32, f32)> {
        let candidates: Vec<&SpawnPoint> = self
            .current
            .spawn_points
            .iter()
            .filter(|sp| sp.team.is_empty() || sp.team.eq_ignore_ascii_case(team))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let i = (self.lcg_unit() * candidates.len() as f32) as usize % candidates.len();
        let sp = candidates[i];
        Some((sp.x, sp.y, sp.z))
    }

    /// 障碍定义列表（TOML 原文，未映射到物理 MapObstacle）
    pub fn obstacles(&self) -> &[ObstacleDef] {
        &self.current.obstacles
    }

    /// 目标定义列表
    pub fn objectives(&self) -> &[ObjectiveDef] {
        &self.current.objectives
    }

    /// 确定性 LCG 单元随机数（与 game.rs 同款常数；本模块独立实现，不跨模块依赖私有函数）
    fn lcg_unit(&self) -> f32 {
        let s = self.seed.get().wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.seed.set(s);
        (s >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// 把 TOML 障碍定义映射为物理侧障碍参数：`(kind, x, z, half_w, half_d)`。
/// 尺寸按盒中心 + 半尺寸转换（`half_w = size.x/2`、`half_d = size.z/2`）；
/// 未知类型回退 Wall 并 `log::warn!`。
pub fn obstacle_to_map_obstacle(def: &ObstacleDef) -> (ObstacleKind, f32, f32, f32, f32) {
    let kind = match def.kind.to_ascii_lowercase().as_str() {
        "wall" => ObstacleKind::Wall,
        "block" => ObstacleKind::Block,
        "barrier" | "cover" => ObstacleKind::Barrier,
        other => {
            log::warn!(
                "map: 未知障碍类型 \"{}\"（位置 {} {} {}），回退为 Wall",
                other,
                def.x,
                def.y,
                def.z
            );
            ObstacleKind::Wall
        }
    };
    (kind, def.x, def.z, def.sx * 0.5, def.sz * 0.5)
}

// ============================================================
// 单测
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 完整示例地图（覆盖中文、注释、多出生点/多障碍/多目标、type 与 kind 同义键）
    const FULL_MAP: &str = r#"# 完整示例地图（行注释应被忽略）
[map]
name = "硫磺岛"
description = "太平洋战场，沙地滩头与碉堡"

spawn_points = [ { x = 10, y = 0, z = 20, team = "blue" }, { x = -10, y = 0, z = 20, team = "red" }, { x = 12, y = 0, z = 22, team = "BLUE" } ]
objectives = [ { id = "flag_a", type = "capture", position = { x = 0, y = 0, z = 0 }, radius = 12.5, capture_time = 8 }, { id = "flag_b", kind = "hold", position = { x = 30, y = 2, z = -30 }, radius = 6, capture_time = 4 } ]
obstacles = [ { type = "wall", position = { x = 5, y = 1, z = 5 }, size = { x = 4, y = 2.4, z = 0.5 } }, { type = "block", position = { x = -5, y = 1, z = -5 }, size = { x = 3, y = 3, z = 3 } }, { type = "cover", position = { x = 0, y = 1, z = -10 }, size = { x = 2, y = 1, z = 1 } } ]
"#;

    /// 写临时 TOML 文件（/tmp，进程号隔离），返回路径
    fn write_tmp(content: &str, tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "steel_front_map_{}_{}.toml",
            tag,
            std::process::id()
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    fn remove_tmp(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn parse_full_map_fields() {
        let data = parse_map_toml(FULL_MAP).unwrap();
        assert_eq!(data.name, "硫磺岛");
        assert_eq!(data.description, "太平洋战场，沙地滩头与碉堡");
        assert_eq!(data.spawn_points.len(), 3);
        assert_eq!(data.objectives.len(), 2);
        assert_eq!(data.obstacles.len(), 3);
    }

    #[test]
    fn parse_spawn_point_details() {
        let data = parse_map_toml(FULL_MAP).unwrap();
        assert_eq!(
            (data.spawn_points[0].x, data.spawn_points[0].y, data.spawn_points[0].z),
            (10.0, 0.0, 20.0)
        );
        assert_eq!(data.spawn_points[0].team, "blue");
        assert_eq!(data.spawn_points[1].team, "red");
        assert_eq!(data.spawn_points[2].team, "BLUE", "原样保留，过滤时忽略大小写");
    }

    #[test]
    fn parse_objective_nested_position() {
        let data = parse_map_toml(FULL_MAP).unwrap();
        let a = &data.objectives[0];
        assert_eq!(a.id, "flag_a");
        assert_eq!(a.kind, "capture", "TOML 键 type 映射到 kind");
        assert_eq!((a.x, a.y, a.z), (0.0, 0.0, 0.0));
        assert!((a.radius - 12.5).abs() < 1e-4);
        assert_eq!(a.capture_time, 8.0);
        let b = &data.objectives[1];
        assert_eq!(b.kind, "hold", "type 缺省时回退 kind 同义键");
        assert_eq!((b.x, b.y, b.z), (30.0, 2.0, -30.0));
    }

    #[test]
    fn parse_obstacle_nested_position_size() {
        let data = parse_map_toml(FULL_MAP).unwrap();
        let o = &data.obstacles[0];
        assert_eq!(o.kind, "wall");
        assert_eq!((o.x, o.y, o.z), (5.0, 1.0, 5.0));
        assert_eq!((o.sx, o.sy, o.sz), (4.0, 2.4, 0.5));
        assert_eq!(data.obstacles[1].kind, "block");
        assert_eq!(data.obstacles[2].kind, "cover");
    }

    #[test]
    fn parse_numbers_negative_decimal() {
        let toml = r#"[map]
spawn_points = [ { x = -3.5, y = -0.25, z = 1e2, team = "red" }, { x = +7, y = 0, z = -12.0, team = "blue" } ]
obstacles = [ { type = "wall", position = { x = -40, y = 0.5, z = 15.25 }, size = { x = 0.5, y = 2.4, z = 0.25 } } ]
"#;
        let data = parse_map_toml(toml).unwrap();
        assert_eq!(data.spawn_points[0].x, -3.5);
        assert_eq!(data.spawn_points[0].y, -0.25);
        assert_eq!(data.spawn_points[0].z, 100.0, "1e2 科学计数");
        assert_eq!(data.spawn_points[1].x, 7.0, "正号");
        assert_eq!(data.spawn_points[1].z, -12.0);
        assert_eq!(data.obstacles[0].x, -40.0);
        assert_eq!(data.obstacles[0].sz, 0.25);
    }

    #[test]
    fn unknown_keys_and_sections_ignored() {
        let toml = r#"[server]
host = "1.2.3.4"
port = 7777

[map]
name = "测试"
foo = "bar"
extra = { a = 1 }
spawn_points = [ { x = 0, y = 0, z = 0, team = "blue", hp = 100, unused = true } ]
obstacles = [ { type = "wall", position = { x = 1, y = 0, z = 1 }, size = { x = 1, y = 1, z = 1 }, damage = 5 } ]
"#;
        let data = parse_map_toml(toml).unwrap();
        assert_eq!(data.name, "测试");
        assert_eq!(data.spawn_points.len(), 1);
        assert_eq!(data.obstacles.len(), 1);
        assert_eq!(data.spawn_points[0].team, "blue");
    }

    #[test]
    fn hash_inside_string_not_comment() {
        let toml = "[map]\nname = \"作战#2\"\nspawn_points = [ { x = 0, y = 0, z = 0, team = \"blue\" } ]\nobstacles = [ { type = \"wall\", position = { x = 0, y = 0, z = 1 }, size = { x = 1, y = 1, z = 1 } } ]\n";
        let data = parse_map_toml(toml).unwrap();
        assert_eq!(data.name, "作战#2", "字符串内的 # 不是注释");
    }

    #[test]
    fn parse_error_missing_bracket_line_number() {
        let toml = "[map]\nname = \"ok\"\nspawn_points = [ { x = 1, y = 0, z = 1, team = \"blue\" \n";
        let e = parse_map_toml(toml).unwrap_err();
        assert!(e.contains("第 3 行"), "错误应带行号: {}", e);
    }

    #[test]
    fn parse_error_missing_quote_line_number() {
        let toml = "[map]\nname = \"未闭合\nspawn_points = [ { x = 1, y = 0, z = 1, team = \"blue\" } ]\n";
        let e = parse_map_toml(toml).unwrap_err();
        assert!(e.contains("第 2 行"), "错误应带行号: {}", e);
    }

    #[test]
    fn load_map_from_temp_file() {
        let path = write_tmp(FULL_MAP, "full");
        let data = load_map(&path.to_str().unwrap()).unwrap();
        remove_tmp(&path);
        assert_eq!(data.name, "硫磺岛");
        assert_eq!(data.spawn_points.len(), 3);
        assert!(!data.obstacles.is_empty());
    }

    #[test]
    fn load_map_validation_errors() {
        // 无出生点
        let p1 = write_tmp(
            "[map]\nname = \"x\"\nobstacles = [ { type = \"wall\", position = { x = 0, y = 0, z = 0 }, size = { x = 1, y = 1, z = 1 } } ]\n",
            "nospawn",
        );
        let e1 = load_map(&p1.to_str().unwrap()).unwrap_err();
        remove_tmp(&p1);
        assert!(e1.contains("spawn_point"), "{}", e1);

        // 无障碍也无目标
        let p2 = write_tmp(
            "[map]\nname = \"x\"\nspawn_points = [ { x = 0, y = 0, z = 0, team = \"blue\" } ]\n",
            "noobj",
        );
        let e2 = load_map(&p2.to_str().unwrap()).unwrap_err();
        remove_tmp(&p2);
        assert!(
            e2.contains("obstacle") || e2.contains("objective"),
            "{}",
            e2
        );

        // 文件不存在
        let e3 = load_map("/tmp/steel_front_does_not_exist_98765.toml").unwrap_err();
        assert!(e3.contains("无法读取"), "{}", e3);
    }

    #[test]
    fn spawn_point_team_filter_and_deterministic() {
        let path = write_tmp(FULL_MAP, "spawn");
        let m1 = MapManager::load(&path.to_str().unwrap()).unwrap();
        let m2 = MapManager::load(&path.to_str().unwrap()).unwrap();
        remove_tmp(&path);

        let blue = m1.spawn_point("blue").unwrap();
        assert!(
            blue == (10.0, 0.0, 20.0) || blue == (12.0, 0.0, 22.0),
            "blue 应落在 blue/BLUE 出生点之一: {:?}",
            blue
        );
        assert_eq!(m1.spawn_point("red").unwrap(), (-10.0, 0.0, 20.0));
        assert!(m1.spawn_point("green").is_none(), "未知阵营应返回 None");

        // 确定性：同图同种子 → 相同出生点序列
        let seq1: Vec<_> = (0..4).map(|_| m1.spawn_point("blue")).collect();
        let seq2: Vec<_> = (0..4).map(|_| m2.spawn_point("blue")).collect();
        assert_eq!(seq1, seq2);
    }

    #[test]
    fn spawn_point_empty_team_matches_any() {
        let toml = "[map]\nspawn_points = [ { x = 1, y = 2, z = 3 } ]\nobstacles = [ { type = \"wall\", position = { x = 0, y = 0, z = 0 }, size = { x = 1, y = 1, z = 1 } } ]\n";
        let path = write_tmp(toml, "emptyteam");
        let m = MapManager::load(&path.to_str().unwrap()).unwrap();
        remove_tmp(&path);
        assert_eq!(m.spawn_point("blue").unwrap(), (1.0, 2.0, 3.0));
        assert_eq!(m.spawn_point("RED").unwrap(), (1.0, 2.0, 3.0));
    }

    #[test]
    fn obstacle_kind_mapping() {
        let cases = [
            ("wall", ObstacleKind::Wall),
            ("block", ObstacleKind::Block),
            ("barrier", ObstacleKind::Barrier),
            ("cover", ObstacleKind::Barrier),
            ("Wall", ObstacleKind::Wall),
        ];
        for (kind, want) in cases {
            let def = ObstacleDef {
                kind: kind.to_string(),
                x: 1.0,
                y: 0.0,
                z: 2.0,
                sx: 4.0,
                sy: 2.4,
                sz: 1.0,
            };
            let (k, x, z, half_w, half_d) = obstacle_to_map_obstacle(&def);
            assert_eq!(k, want, "kind {}", kind);
            assert_eq!((x, z), (1.0, 2.0));
            assert_eq!((half_w, half_d), (2.0, 0.5), "half = size/2");
        }
        // 未知类型回退 Wall
        let def = ObstacleDef {
            kind: "bunker".to_string(),
            x: 1.0,
            y: 0.0,
            z: 2.0,
            sx: 4.0,
            sy: 2.4,
            sz: 1.0,
        };
        let (k, ..) = obstacle_to_map_obstacle(&def);
        assert_eq!(k, ObstacleKind::Wall);
    }

    #[test]
    fn reload_changes_data() {
        let p1 = write_tmp(FULL_MAP, "r1");
        let p2 = write_tmp(
            "[map]\nname = \"第二个地图\"\nspawn_points = [ { x = -5, y = 0, z = -5, team = \"blue\" } ]\nobstacles = [ { type = \"block\", position = { x = 9, y = 0, z = 9 }, size = { x = 2, y = 2, z = 2 } } ]\n",
            "r2",
        );
        let mut m = MapManager::load(&p1.to_str().unwrap()).unwrap();
        assert_eq!(m.data().name, "硫磺岛");
        m.reload(&p2.to_str().unwrap()).unwrap();
        assert_eq!(m.data().name, "第二个地图");
        assert_eq!(m.data().spawn_points.len(), 1);
        assert_eq!(m.spawn_point("blue").unwrap(), (-5.0, 0.0, -5.0));
        remove_tmp(&p1);
        remove_tmp(&p2);
    }

    #[test]
    fn map_manager_new_is_empty() {
        let m = MapManager::new();
        assert!(m.data().spawn_points.is_empty());
        assert!(m.obstacles().is_empty());
        assert!(m.objectives().is_empty());
        assert!(m.spawn_point("blue").is_none());
    }

    #[test]
    fn map_block_height_constant_locked() {
        // 主会话接线时以该常量组装物理障碍高度，锁定数值防漂移
        assert_eq!(MAP_BLOCK_HEIGHT, 2.4);
    }

    /// 跨行内联数组：spawn_points/obstacles 换行书写必须能解析（示例地图用多行格式）
    #[test]
    fn multiline_array_values_parse() {
        let toml = r#"[map]
name = "多行测试"
spawn_points = [
    { x = -30.0, y = 1.0, z = -40.0, team = "blue" },
    { x = 30.0, y = 1.0, z = -40.0, team = "red" },
]
obstacles = [
    { type = "wall", position = { x = -10.0, y = 1.5, z = 0.0 }, size = { x = 1.0, y = 3.0, z = 20.0 } },
]
objectives = [
    { id = "A", type = "capture", position = { x = 0.0, y = 0.0, z = -15.0 }, radius = 5.0, capture_time = 10.0 },
]
"#;
        let data = parse_map_toml(toml).expect("多行数组应可解析");
        assert_eq!(data.name, "多行测试");
        assert_eq!(data.spawn_points.len(), 2);
        assert_eq!(data.spawn_points[0].team, "blue");
        assert_eq!(data.obstacles.len(), 1);
        assert_eq!(data.objectives.len(), 1);
        assert_eq!(data.objectives[0].id, "A");
    }

    /// [rule] 节解析：capture/kill/time 三种规则字段
    #[test]
    fn rule_section_parses() {
        let toml = r#"[map]
name = "规则测试"
spawn_points = [ { x = 0, y = 0, z = 0, team = "blue" } ]
obstacles = [ { type = "wall", position = { x = 1, y = 0, z = 1 }, size = { x = 1, y = 1, z = 1 } } ]
[rule]
kind = "kill"
target = 30
"#;
        let data = parse_map_toml(toml).expect("含 rule 节应可解析");
        assert_eq!(data.rule.kind, "kill");
        assert_eq!(data.rule.target, 30);
        assert_eq!(data.rule.required, 0);

        let toml2 = r#"[map]
name = "规则测试2"
spawn_points = [ { x = 0, y = 0, z = 0, team = "blue" } ]
obstacles = [ { type = "wall", position = { x = 1, y = 0, z = 1 }, size = { x = 1, y = 1, z = 1 } } ]
[rule]
kind = "capture"
required = 2
"#;
        let data2 = parse_map_toml(toml2).unwrap();
        assert_eq!(data2.rule.kind, "capture");
        assert_eq!(data2.rule.required, 2);
    }

    /// 关卡列表 index.toml 解析（maps 字符串数组）
    #[test]
    fn map_list_index_parses() {
        let dir = std::env::temp_dir().join(format!("sf_index_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = dir.join("index.toml");
        std::fs::write(&index, "maps = [\n    \"a.toml\",\n    \"b.toml\",\n]\n").unwrap();
        let list = load_map_list(&index.to_string_lossy()).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].ends_with("a.toml"), "相对路径应拼接目录: {}", list[0]);
        assert!(list[1].ends_with("b.toml"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
