//! AI 控制面（引擎化 P5，见 `docs/ENGINE-PLAN.md`）：本机 TCP 上的**行长文本协议**。
//!
//! 存在的理由（用户原话）："保留原生 AI 接口和操作，AI 可以通过接口和插件无缝控制，
//! 并且获得实时数据反馈"。所以这个模块只做三件事，且都做到可被机器解析：
//!
//! 1. **命令**：`CMD <名字> [参数...]` —— 走的就是 `PluginRegistry::run_command`，
//!    **与编辑器按钮是同一张表、同一个函数**（铁律 4：人能做的 AI 也能做，反之亦然）；
//! 2. **上报**：`SUB` 之后每 `TELEMETRY_PERIOD` 秒推一行 `TEL key=value ...`；
//! 3. **自描述**：连上立刻发 `HELLO`，含协议版本与命令条数 —— AI 不需要预先知道能力清单。
//!
//! ## 为什么用通道而不是共享状态
//!
//! 游戏状态（`Scene` / `Game`）**只在主线程**。网络线程如果直接持有 `&mut` 就得加锁，
//! 而主线程每帧要跑 16ms 的渲染 —— 任何一把锁都会变成帧时间抖动，甚至死锁（渲染里再取锁）。
//! 所以：网络线程只做**字符串进出**，把请求塞进 `mpsc`，主线程每帧 drain 一次并把结果送回。
//! 这同时保证了"命令与编辑器按钮在同一线程、同一时序上执行"，行为不会因为来源不同而变。
//!
//! ## 安全边界（默认关）
//!
//! 只有设了 `RV3D_AIPC=<端口>` 才监听，且**只绑 127.0.0.1**（不对外网开放）。
//! 默认关是有意的：一个能改场景、能存盘的接口不该在某次忘了关的状态下被局域网碰到。

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// 协议版本（改动语义时自增；AI 侧据此判断兼容）。
pub const PROTOCOL_VERSION: u32 = 1;
/// 上报周期（秒）。
pub const TELEMETRY_PERIOD: f32 = 0.25;

/// 一条来自客户端的命令请求（网络线程 → 主线程）。
pub struct AiRequest {
    /// 命令名（与 `PluginRegistry` 里注册的完全一致）。
    pub name: String,
    /// 空格切开的参数。
    pub args: Vec<String>,
    /// 回执通道：主线程执行完把 `OK ...` / `ERR ...` 写回去。
    pub reply: Sender<String>,
}

/// 服务器句柄。`Drop` 时置停止位并叫醒监听（不 join 线程：进程退出时它自己会结束）。
pub struct AiServer {
    pub port: u16,
    stop: Arc<AtomicBool>,
    /// 待执行命令的接收端（主线程持有）。
    requests: Receiver<AiRequest>,
    /// 上报行（主线程每帧 push，网络线程转发给接收方）。
    telemetry: Arc<Mutex<Vec<String>>>,
}

impl AiServer {
    /// 绑定并起线程。`port = 0` 表示由系统分配（测试用），实际端口见返回值。
    pub fn start(port: u16) -> Result<AiServer, String> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("AIPC 监听失败 127.0.0.1:{port}：{e}"))?;
        let actual = listener
            .local_addr()
            .map_err(|e| format!("取端口失败：{e}"))?
            .port();
        // 非阻塞 + 短超时轮询：不 join、不留 join handle，进程退出即终止（daemon 语义）。
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("设置非阻塞失败：{e}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let telemetry: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel::<AiRequest>();
        {
            let stop = stop.clone();
            let telemetry = telemetry.clone();
            thread::Builder::new()
                .name("aipc".to_string())
                .spawn(move || accept_loop(listener, stop, tx, telemetry))
                .map_err(|e| format!("AIPC 线程启动失败：{e}"))?;
        }
        log::info!("aipc: 监听 127.0.0.1:{actual}（协议 v{PROTOCOL_VERSION}）");
        Ok(AiServer {
            port: actual,
            stop,
            requests: rx,
            telemetry,
        })
    }

    /// 主线程每帧调用：把排队请求交给 `exec` 执行，并把回执写回客户端。
    ///
    /// `exec` 的返回值就是回给客户端的正文（`OK <正文>` / `ERR <正文>`）。
    pub fn pump<F>(&mut self, mut exec: F) -> usize
    where
        F: FnMut(&str, &[String]) -> Result<String, String>,
    {
        let mut n = 0;
        while let Ok(req) = self.requests.try_recv() {
            let line = match exec(&req.name, &req.args) {
                Ok(msg) => format!("OK {msg}"),
                Err(e) => format!("ERR {e}"),
            };
            // 客户端可能已经断开：回执发不出去不算错误（网络线程会忽略）。
            let _ = req.reply.send(line);
            n += 1;
            if n >= MAX_REQUESTS_PER_FRAME {
                break;
            }
        }
        n
    }

    /// 主线程推送一行上报（网络线程会在下个周期发给接收方）。
    ///
    /// 🔴 **只保留最新一行**，不是排队。理由：排队会让客户端收到**过期状态**——
    /// 实测过一次"命令返回 entities=3、上报还报 entities=2"，因为队列里躺着上一帧的行。
    /// 上报的语义是"当前状态"，过期的那几行没有价值，反而会让人以为操作没生效。
    /// （需要历史记录请让客户端自己记，服务端只保证"最近"。）
    pub fn push_telemetry(&self, line: impl Into<String>) {
        if let Ok(mut q) = self.telemetry.lock() {
            q.clear();
            q.push(line.into());
        }
    }
}

impl Drop for AiServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 每帧最多处理多少条请求：防止某个客户端刷屏把一帧的时间全占了（渲染掉帧比"命令慢一拍"更糟）。
const MAX_REQUESTS_PER_FRAME: usize = 32;

fn accept_loop(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    tx: Sender<AiRequest>,
    telemetry: Arc<Mutex<Vec<String>>>,
) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _addr)) => {
                let tx = tx.clone();
                let stop2 = stop.clone();
                let tel = telemetry.clone();
                // 每个连接一个线程：客户端数量是个位数（编辑器 + AI），不需要事件循环。
                thread::Builder::new()
                    .name("aipc-conn".to_string())
                    .spawn(move || serve_conn(stream, stop2, tx, tel))
                    .ok();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}

fn serve_conn(
    stream: TcpStream,
    stop: Arc<AtomicBool>,
    tx: Sender<AiRequest>,
    telemetry: Arc<Mutex<Vec<String>>>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let peer = stream.peer_addr().ok();
    let reader_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut writer = stream;
    let mut reader = BufReader::new(reader_stream);
    let mut subscribed = false;
    let mut last_push = Instant::now();
    let mut line = String::new();
    if writer
        .write_all(
            format!(
                "HELLO v{PROTOCOL_VERSION} proto=line cmd=CMD <name> [args...] sub=SUB quit=QUIT\n"
            )
            .as_bytes(),
        )
        .is_err()
    {
        return;
    }
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let cmd = line.trim();
                if cmd.is_empty() {
                    continue;
                }
                let mut it = cmd.split_whitespace();
                let verb = it.next().unwrap_or("");
                match verb.to_ascii_uppercase().as_str() {
                    "QUIT" => break,
                    "SUB" => {
                        subscribed = true;
                        last_push = Instant::now();
                        let _ = writer.write_all(b"OK subscribed\n");
                    }
                    "UNSUB" => {
                        subscribed = false;
                        let _ = writer.write_all(b"OK unsubscribed\n");
                    }
                    "CMD" => {
                        let name = it.next().unwrap_or("").to_string();
                        let args: Vec<String> = it.map(|s| s.to_string()).collect();
                        let (rtx, rrx) = mpsc::channel::<String>();
                        if tx
                            .send(AiRequest {
                                name,
                                args,
                                reply: rtx,
                            })
                            .is_err()
                        {
                            let _ = writer.write_all("ERR 主线程已退出\n".as_bytes());
                            break;
                        }
                        // 等主线程回执（有界等待：主线程卡住时不能让连接永远挂着）。
                        match rrx.recv_timeout(Duration::from_millis(500)) {
                            Ok(resp) => {
                                let _ = writer.write_all(resp.as_bytes());
                                let _ = writer.write_all(b"\n");
                            }
                            Err(_) => {
                                let _ = writer.write_all("ERR 主线程未在 500ms 内回执\n".as_bytes());
                            }
                        }
                    }
                    other => {
                        let _ = writer
                            .write_all(format!("ERR 未知动词 {other}（可用 CMD/SUB/UNSUB/QUIT）\n").as_bytes());
                    }
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // 读超时是正常的：下面照常做上报投递。
            }
            Err(_) => break,
        }
        if subscribed && last_push.elapsed().as_secs_f32() >= TELEMETRY_PERIOD {
            last_push = Instant::now();
            let lines: Vec<String> = telemetry
                .lock()
                .map(|mut q| std::mem::take(&mut *q))
                .unwrap_or_default();
            for l in lines {
                if writer.write_all(format!("TEL {l}\n").as_bytes()).is_err() {
                    return;
                }
            }
        }
    }
    if let Some(p) = peer {
        log::info!("aipc: 客户端断开 {p}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream as Client;


    /// 🔴 上报**只留最新一行**：排队会让客户端读到过期状态。
    /// 实测过一次"命令返回 entities=3、上报报 entities=2"。
    ///
    /// 反向验证：把 `q.clear()` 删掉（改回排队）⇒ 客户端先收到旧的 1，红。
    #[test]
    fn telemetry_keeps_only_the_latest_line() {
        let server = AiServer::start(0).expect("应能绑定");
        let addr = format!("127.0.0.1:{}", server.port);
        let mut c = Client::connect(&addr).expect("应能连上");
        c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let mut line = String::new();
        r.read_line(&mut line).unwrap(); // HELLO
        c.write_all(b"SUB\n").unwrap();
        line.clear();
        r.read_line(&mut line).unwrap();
        assert!(line.starts_with("OK"), "开启上报回执：{line:?}");
        // 主线程连推三行，客户端只该看到最后一行
        server.push_telemetry("entities=1");
        server.push_telemetry("entities=2");
        server.push_telemetry("entities=3");
        line.clear();
        r.read_line(&mut line).unwrap();
        assert_eq!(
            line.trim(),
            "TEL entities=3",
            "只该收到最新一行（排队会先给 entities=1）：{line:?}"
        );
    }

    /// 连上 → HELLO → CMD → 主线程回执。**整条链路都测**（含线程、通道、协议）。
    ///
    /// 反向验证：把 `serve_conn` 里 `tx.send` 那行删掉 ⇒ 客户端收不到 `OK`，这条红。
    #[test]
    fn aipc_roundtrip_executes_commands_via_main_thread_channel() {
        let mut server = AiServer::start(0).expect("应能绑定 127.0.0.1:0");
        let addr = format!("127.0.0.1:{}", server.port);
        let mut c = Client::connect(&addr).expect("应能连上");
        c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert!(
            line.starts_with("HELLO v1"),
            "连上必须先收到 HELLO，实际 {line:?}"
        );
        // 客户端发命令；主线程用 pump 处理（这里用"主线程"的角色自己模拟）
        let mut w = c;
        w.write_all(b"CMD scene.count\n").unwrap();
        // 服务线程把请求放进通道，主线程这一刻才执行
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while seen.is_empty() && Instant::now() < deadline {
            server.pump(|name, args| {
                seen.push((name.to_string(), args.len()));
                Ok(format!("{} args={}", name, args.len()))
            });
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(seen, vec![("scene.count".to_string(), 0)], "主线程应收到命令");
        line.clear();
        r.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "OK scene.count args=0");
    }

    /// 未知动词必须回 ERR 并保持连接（不能让客户端一写错命令就断线）。
    #[test]
    fn aipc_reports_unknown_verb_without_dropping_connection() {
        let _server = AiServer::start(0).expect("应能绑定");
        let addr = format!("127.0.0.1:{}", _server.port);
        let mut c = Client::connect(&addr).expect("应能连上");
        c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let mut line = String::new();
        r.read_line(&mut line).unwrap(); // HELLO
        c.write_all(b"FROB\n").unwrap();
        line.clear();
        r.read_line(&mut line).unwrap();
        assert!(line.starts_with("ERR 未知动词 FROB"), "实际 {line:?}");
        // 连接仍然活着：再发一个合法动词能收到回复
        c.write_all(b"UNSUB\n").unwrap();
        line.clear();
        r.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "OK unsubscribed");
    }
}
