//! LLM 战时指挥官（llama.cpp llama-server 集成，零第三方依赖）。
//!
//! 定位：LLM 只做「战役级战略决策」——每 RV3D_LLM_INTERVAL 秒（默认 90s）接收
//! 战场态势 JSON，输出各连命令 JSON（严格 schema），由 ai_command 执行层分解到
//! 班/人。与游戏主循环零耦合：独立线程 HTTP 调用 + 双缓冲，超时/断连/格式非法
//! 一律回退内置启发式司令（玩家无感）。
//!
//! 启用：RV3D_LLM=1（默认 http://127.0.0.1:8080）或 RV3D_LLM=http://host:port；
//! 关闭：RV3D_LLM=0/off。RV3D_LLM_INTERVAL=秒覆盖决策周期（测试用 5s）。
//!
//! llama-server 启动（scripts/start_llm_server.ps1）：
//!   llama-server.exe -m <model.gguf> -c 4096 --host 127.0.0.1 --port 8080

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LlmOrder {
    Assault,
    Hold,
    FlankL,
    FlankR,
    Regroup,
}

impl LlmOrder {
    pub fn label(&self) -> &'static str {
        match self {
            LlmOrder::Assault => "Assault",
            LlmOrder::Hold => "Hold",
            LlmOrder::FlankL => "FlankL",
            LlmOrder::FlankR => "FlankR",
            LlmOrder::Regroup => "Regroup",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompanyCmd {
    pub order: LlmOrder,
    pub x: f32,
    pub z: f32,
}

struct Shared {
    situation: Mutex<String>,
    latest: Mutex<Option<Vec<CompanyCmd>>>,
    stopped: AtomicBool,
}

pub struct LlmCommander {
    shared: Arc<Shared>,
    #[allow(dead_code)] // 线程句柄（持活线程；析构随进程回收）
    handle: Option<std::thread::JoinHandle<()>>,
}

// ============================================================
// 迷你 JSON 解析（对象/数组/字符串/数字/布尔/null 子集）
// ============================================================
#[allow(dead_code)] // 解析器完整实现保留（Bool 值当前未读）
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

fn parse_json_fn(s: &str) -> Result<Json, String> {
    let mut p = P { s: s.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("JSON 尾部多余字符 @{}", p.i));
    }
    Ok(v)
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] == b' ' || self.s[self.i] == b'\t' || self.s[self.i] == b'\n' || self.s[self.i] == b'\r') {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        let c = *self.s.get(self.i).ok_or("JSON 意外结束")?;
        match c {
            b'{' => self.obj(),
            b'[' => self.arr(),
            b'"' => Ok(Json::Str(self.string()?)),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'n' => self.lit("null", Json::Null),
            b'-' | b'0'..=b'9' => self.num(),
            _ => Err(format!("JSON 意外字符 '{}'", c as char)),
        }
    }
    fn lit(&mut self, lit: &str, v: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            Ok(v)
        } else {
            Err("JSON 字面量错误".into())
        }
    }
    fn num(&mut self) -> Result<Json, String> {
        let st = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_digit()
                || self.s[self.i] == b'-'
                || self.s[self.i] == b'+'
                || self.s[self.i] == b'.'
                || self.s[self.i] == b'e'
                || self.s[self.i] == b'E')
        {
            self.i += 1;
        }
        let txt = String::from_utf8_lossy(&self.s[st..self.i]).to_string();
        txt.parse::<f64>().map(Json::Num).map_err(|_| "JSON 数字解析失败".into())
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = *self.s.get(self.i).ok_or("字符串未闭合")?;
            self.i += 1;
            if c == b'"' {
                break;
            }
            if c == b'\\' {
                let e = *self.s.get(self.i).ok_or("转义未闭合")?;
                self.i += 1;
                out.push(match e {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'u' => {
                        let h = String::from_utf8_lossy(&self.s[self.i..self.i + 4]).to_string();
                        self.i += 4;
                        let cp = u32::from_str_radix(&h, 16).map_err(|_| "unicode 转义非法")?;
                        char::from_u32(cp).ok_or("unicode 码点非法")?
                    }
                    _ => return Err("未知转义".into()),
                });
            } else if c < 0x20 {
                return Err("控制字符非法".into());
            } else {
                out.push(c as char);
            }
        }
        Ok(out)
    }
    fn obj(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut kv = Vec::new();
        loop {
            self.ws();
            if *self.s.get(self.i).ok_or("对象未闭合")? == b'}' {
                self.i += 1;
                break;
            }
            let k = self.string()?;
            self.ws();
            if *self.s.get(self.i).ok_or("缺少冒号")? != b':' {
                return Err("缺少冒号".into());
            }
            self.i += 1;
            let v = self.value()?;
            kv.push((k, v));
            self.ws();
            match *self.s.get(self.i).ok_or("对象未闭合")? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    break;
                }
                _ => return Err("对象分隔符错误".into()),
            }
        }
        Ok(Json::Obj(kv))
    }
    fn arr(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut items = Vec::new();
        loop {
            self.ws();
            if *self.s.get(self.i).ok_or("数组未闭合")? == b']' {
                self.i += 1;
                break;
            }
            let v = self.value()?;
            items.push(v);
            self.ws();
            match *self.s.get(self.i).ok_or("数组未闭合")? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    break;
                }
                _ => return Err("数组分隔符错误".into()),
            }
        }
        Ok(Json::Arr(items))
    }
}

// ============================================================
// 最小 HTTP/1.1 客户端（POST，超时可控）
// ============================================================
fn http_post_json(url: &str, body: &str, timeout: Duration) -> Result<String, String> {
    let (host, port, path) = parse_url(url)?;
    let addr = format!("{host}:{port}");
    let tcp = std::net::TcpStream::connect_timeout(
        &addr.parse().map_err(|e| format!("地址解析失败 {addr}: {e}"))?,
        timeout,
    )
    .map_err(|e| format!("连接 {addr} 失败: {e}"))?;
    let _ = tcp.set_read_timeout(Some(timeout));
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let mut stream = tcp;
    {
        use std::io::Write;
        let _ = stream.write_all(req.as_bytes());
        let _ = stream.flush();
    }
    let mut buf = Vec::new();
    {
        use std::io::Read;
        let _ = stream.read_to_end(&mut buf);
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let body_start = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
    Ok(text[body_start.min(text.len())..].to_string())
}

fn parse_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.trim_start_matches("http://").trim_start_matches("https://");
    let (hp, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/v1/chat/completions".to_string()),
    };
    let (host, port) = match hp.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|e| format!("端口非法: {e}"))?),
        None => (hp.to_string(), 80),
    };
    let path = if path.is_empty() { "/v1/chat/completions".to_string() } else { path };
    Ok((host, port, path))
}

// ============================================================
// 提示词与校验
// ============================================================
fn system_prompt() -> &'static str {
    "你是现代步兵营指挥官，收到战场态势 JSON 后为每个连队下达命令。\n规则：\n- 只输出 JSON，禁止任何解释、思考或代码围栏；\n- order 只能是 Assault/Hold/FlankL/FlankR/Regroup；\n- x/z 是世界坐标 [-270,270]；\n- 一个连队一条命令，数量与态势一致。\n示例输入：{\"companies\":[{\"id\":0,\"strength\":30,\"x\":100,\"z\":-50}]}\n示例输出：{\"companies\":[{\"order\":\"Assault\",\"x\":120,\"z\":-30}]}"
}

pub fn parse_company_cmds(content: &str, n_companies: usize) -> Result<Vec<CompanyCmd>, String> {
    let json = parse_json_fn(content).map_err(|e| format!("JSON 解析失败: {e}"))?;
    let arr = json.get("companies").and_then(|j| j.as_arr()).ok_or("缺少 companies 数组")?;
    if arr.is_empty() {
        return Err("companies 为空".into());
    }
    if arr.len() != n_companies {
        return Err(format!("companies 数量 {} != {}", arr.len(), n_companies));
    }
    let mut out = Vec::with_capacity(n_companies);
    for (i, item) in arr.iter().enumerate() {
        let order_s = item.get("order").and_then(|j| j.as_str()).ok_or(format!("连 {i} 缺 order"))?;
        let order = match order_s {
            "Assault" => LlmOrder::Assault,
            "Hold" => LlmOrder::Hold,
            "FlankL" => LlmOrder::FlankL,
            "FlankR" => LlmOrder::FlankR,
            "Regroup" => LlmOrder::Regroup,
            _ => return Err(format!("连 {i} order 非法: {order_s}")),
        };
        let x = item.get("x").and_then(|j| j.as_f64()).ok_or(format!("连 {i} 缺 x"))?;
        let z = item.get("z").and_then(|j| j.as_f64()).ok_or(format!("连 {i} 缺 z"))?;
        if !(x.is_finite() && z.is_finite()) || x.abs() > 270.0 || z.abs() > 270.0 {
            return Err(format!("连 {i} 目标点越界: ({x:.1},{z:.1})"));
        }
        out.push(CompanyCmd { order, x: x as f32, z: z as f32 });
    }
    Ok(out)
}

// ============================================================
// 指挥官线程
// ============================================================
impl LlmCommander {
    pub fn from_env() -> Option<LlmCommander> {
        let url = match std::env::var("RV3D_LLM") {
            Ok(v) if v == "0" || v == "off" || v.is_empty() => return None,
            Ok(v) if v == "1" => "http://127.0.0.1:8080".to_string(),
            Ok(v) => v.trim().to_string(),
            Err(_) => return None,
        };
        let interval = std::env::var("RV3D_LLM_INTERVAL")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|v| *v >= 1.0)
            .unwrap_or(90.0);
        Some(LlmCommander::start(url, interval))
    }

    pub fn start(url: String, interval: f32) -> LlmCommander {
        log::info!("llmcmd: LLM 指挥官启动 url={url} interval={interval}s");
        let shared = Arc::new(Shared {
            situation: Mutex::new(String::new()),
            latest: Mutex::new(None),
            stopped: AtomicBool::new(false),
        });
        let sh = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("llm-commander".into())
            .spawn(move || {
                let mut next_run = Instant::now() + Duration::from_millis((interval * 1000.0) as u64);
                while !sh.stopped.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(250));
                    if Instant::now() < next_run {
                        continue;
                    }
                    next_run = Instant::now() + Duration::from_millis((interval * 1000.0) as u64);
                    let situation = sh.situation.lock().map(|s| s.clone()).unwrap_or_default();
                    if situation.trim().is_empty() {
                        continue;
                    }
                    let body = format!(
                        "{{\"model\":\"local\",\"temperature\":0.4,\"max_tokens\":600,\"no_think\":true,\"messages\":[{{\"role\":\"system\",\"content\":\"{}\"}},{{\"role\":\"user\",\"content\":\"{}\"}}]}}",
                        system_prompt().replace('\n', "\\n").replace('"', "\\\""),
                        situation.replace('\n', "\\n").replace('"', "\\\""),
                    );
                    match http_post_json(&url, &body, Duration::from_secs(150)) {
                        Ok(resp) => {
                            // 双字段提取：部分模板把答案放 content，部分放 reasoning_content
                            let content = parse_json_fn(&resp)
                                .ok()
                                .and_then(|j| {
                                    let msg = j.get("choices")?.as_arr()?.first()?.get("message")?;
                                    let c = msg
                                        .get("content")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("");
                                    let r = msg
                                        .get("reasoning_content")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("");
                                    Some(format!("{c}\n{r}"))
                                })
                                .unwrap_or_default();
                            // 剥离围栏/解释：截取第一个 { 到最后一个 }
                            let content = content.replace("\r\n", "\n");
                            let content = {
                                let b = content.find('{');
                                let e = content.rfind('}');
                                match (b, e) {
                                    (Some(b), Some(e)) if e > b => content[b..=e].to_string(),
                                    _ => content.trim().to_string(),
                                }
                            };
                            if content.starts_with('{') {
                                // 解析校验
                                let n = situation.split("\"companies\"").count().max(1) - 1;
                                let n = if n > 0 { n } else { 3 };
                                match parse_company_cmds(&content, n) {
                                    Ok(cmds) => {
                                        log::info!(
                                            "llmcmd: 命令已采纳 {:?}",
                                            cmds.iter().map(|c| (c.order.label(), c.x as i32, c.z as i32)).collect::<Vec<_>>()
                                        );
                                        if let Ok(mut m) = sh.latest.lock() {
                                            *m = Some(cmds);
                                        }
                                    }
                                    Err(e) => log::warn!("llmcmd: 命令校验失败: {e}"),
                                }
                            } else {
                                log::warn!(
                                    "llmcmd: 输出非 JSON（{} 字符，头部: {}）",
                                    content.len(),
                                    &content[..content.len().min(80)]
                                );
                            }
                        }
                        Err(e) => log::warn!("llmcmd: HTTP 失败: {e}"),
                    }
                }
            })
            .ok();
        LlmCommander { shared, handle }
    }

    pub fn push_situation(&self, situation: &str) {
        if let Ok(mut s) = self.shared.situation.lock() {
            *s = situation.to_string();
        }
    }

    pub fn take_latest(&self) -> Option<Vec<CompanyCmd>> {
        self.shared.latest.lock().ok().and_then(|m| m.clone())
    }
}
