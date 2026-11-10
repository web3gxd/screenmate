#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod daemon;
use serde_json::{json, Value};
use std::{env, fs, io::{self, BufRead, Read, Write}, net::{TcpStream, SocketAddrV4, Ipv4Addr}, path::PathBuf, time::Duration, thread};

fn schema() -> Vec<Value> {
    let empty=json!({"type":"object","properties":{},"additionalProperties":false});
    vec![
      json!({"name":"look","description":"Свежий снимок экрана и информация об изменении.","inputSchema":{"type":"object","properties":{"monitor":{"type":"string","description":"ID монитора из list_windows"},"region":{"type":"object","properties":{"x":{"type":"integer"},"y":{"type":"integer"},"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1}},"required":["x","y","width","height"],"additionalProperties":false}},"additionalProperties":false}}),
      json!({"name":"look_diff","description":"Свежий снимок, только если экран изменился.","inputSchema":empty}),
      json!({"name":"watch","description":"Наблюдать не более 60 секунд и вернуть последний из не более пяти снимков.","inputSchema":{"type":"object","properties":{"seconds":{"type":"number","minimum":0,"maximum":60},"interval":{"type":"number","minimum":0.2,"maximum":60}},"required":["seconds","interval"],"additionalProperties":false}}),
      json!({"name":"list_windows","description":"Список мониторов и окон.","inputSchema":empty}),
      json!({"name":"inbox","description":"Получить непрочитанные сообщения и отметить их прочитанными.","inputSchema":empty}),
      json!({"name":"say","description":"Вывод ответа в приложении временно отключён.","inputSchema":{"type":"object","properties":{"text":{"type":"string","minLength":1,"maxLength":20000}},"required":["text"],"additionalProperties":false}}),
    ]
}
fn session_path() -> Result<PathBuf,String> { Ok(PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("Нет LOCALAPPDATA")?).join("Screenmate").join("session.json")) }

const MAX_HTTP_BODY:usize=20*1024*1024;
const MAX_HTTP_HEADERS:usize=16*1024;
const MAX_HTTP_LINE:usize=8*1024;
const MAX_CHUNK_METADATA:usize=1024*1024;

// Limit allocations even if the peer never sends a newline. Every HTTP line
// is required to end in CRLF to avoid ambiguous parsing.
fn http_line<R:BufRead>(reader:&mut R,limit:usize)->Result<Vec<u8>,String> {
    let mut line=Vec::new();
    let n=reader.take((limit+1) as u64).read_until(b'\n',&mut line).map_err(|_|"Ошибка чтения HTTP-ответа.")?;
    if n==0 || n>limit || !line.ends_with(b"\r\n") {return Err("Некорректная строка HTTP-ответа.".into());}
    Ok(line)
}

fn http_chunks<R:BufRead>(reader:&mut R,body_limit:usize)->Result<Vec<u8>,String> {
    let mut body=Vec::new();let mut metadata=0usize;
    loop {
        let line=http_line(reader,MAX_HTTP_LINE)?;
        metadata=metadata.checked_add(line.len()).ok_or("Слишком много метаданных ответа.")?;
        if metadata>MAX_CHUNK_METADATA {return Err("Слишком много метаданных ответа.".into());}
        let raw=&line[..line.len()-2];
        let digits=raw.split(|&b|b==b';').next().unwrap_or_default();
        if digits.is_empty() || digits.len()>16 || !digits.iter().all(u8::is_ascii_hexdigit) {return Err("Некорректный размер чанка HTTP.".into());}
        let digits=std::str::from_utf8(digits).map_err(|_|"Некорректный размер чанка HTTP.")?;
        let size=usize::from_str_radix(digits,16).map_err(|_|"Некорректный размер чанка HTTP.")?;
        if size==0 {
            // Consume and bound the optional trailer section; it is not used.
            let mut trailers=0usize;
            loop {
                let line=http_line(reader,MAX_HTTP_LINE)?;
                trailers=trailers.checked_add(line.len()).ok_or("Слишком большие трейлеры HTTP.")?;
                if trailers>MAX_HTTP_HEADERS {return Err("Слишком большие трейлеры HTTP.".into());}
                if line==b"\r\n" {return Ok(body);}
                if !line.contains(&b':') {return Err("Некорректный трейлер HTTP.".into());}
            }
        }
        let end=body.len().checked_add(size).filter(|&n|n<=body_limit).ok_or("Некорректный размер ответа.")?;
        let start=body.len();body.resize(end,0);
        reader.read_exact(&mut body[start..]).map_err(|_|"Неполный чанк HTTP.")?;
        let mut separator=[0u8;2];reader.read_exact(&mut separator).map_err(|_|"Неполный чанк HTTP.")?;
        if separator!=*b"\r\n" {return Err("Некорректный разделитель чанка HTTP.".into());}
    }
}

/// Read a single response using either Content-Length or Transfer-Encoding:
/// chunked. Reject conflicting/unsupported framing and cap all allocations.
fn http_response<R:BufRead>(reader:&mut R,body_limit:usize)->Result<(bool,Vec<u8>),String> {
    let status=http_line(reader,MAX_HTTP_LINE)?;
    let status=std::str::from_utf8(&status[..status.len()-2]).map_err(|_|"Некорректный HTTP-ответ.")?;
    let mut fields=status.split_whitespace();let version=fields.next().unwrap_or("");let code=fields.next().unwrap_or("");
    if !["HTTP/1.0","HTTP/1.1"].contains(&version) || code.len()!=3 || !code.bytes().all(|b|b.is_ascii_digit()) {return Err("Некорректный HTTP-статус.".into());}
    let status_ok=code=="200";
    let mut length=None;let mut chunked=false;let mut header_bytes=status.len()+2;
    loop {
        let line=http_line(reader,MAX_HTTP_LINE)?;
        header_bytes=header_bytes.checked_add(line.len()).ok_or("Слишком большие заголовки HTTP.")?;
        if header_bytes>MAX_HTTP_HEADERS {return Err("Слишком большие заголовки HTTP.".into());}
        if line==b"\r\n" {break;}
        let line=std::str::from_utf8(&line[..line.len()-2]).map_err(|_|"Некорректный HTTP-заголовок.")?;
        let (name,value)=line.split_once(':').ok_or("Некорректный HTTP-заголовок.")?;
        if name.eq_ignore_ascii_case("content-length") {
            let n=value.trim().parse::<usize>().map_err(|_|"Некорректный размер ответа.")?;
            if length.is_some_and(|previous|previous!=n) {return Err("Несогласованные размеры ответа.".into());}
            length=Some(n);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {return Err("Неподдерживаемое кодирование HTTP.".into());}
            chunked=true;
        }
    }
    if chunked && length.is_some() {return Err("Неоднозначный размер ответа HTTP.".into());}
    let data=if chunked {http_chunks(reader,body_limit)?} else {
        let length=length.filter(|n|*n<=body_limit).ok_or("Некорректный размер ответа.")?;
        let mut data=vec![0;length];reader.read_exact(&mut data).map_err(|_|"Неполный ответ приложения.")?;data
    };
    Ok((status_ok,data))
}

fn rpc(method: &str, params: &Value) -> Result<Value,String> {
    let session:Value=serde_json::from_slice(&fs::read(session_path()?).map_err(|_|"Скринмейт не запущен или файл сеанса недоступен.")?).map_err(|_|"Некорректный файл сеанса.")?;
    let port=session["port"].as_u64().and_then(|p|u16::try_from(p).ok()).filter(|p|*p!=0).ok_or("Некорректный порт сеанса.")?;
    let token=session["token"].as_str().filter(|t| (32..=256).contains(&t.len()) && t.bytes().all(|c| c.is_ascii_alphanumeric()||c==b'_'||c==b'-')).ok_or("Некорректный токен сеанса.")?;
    let address=SocketAddrV4::new(Ipv4Addr::LOCALHOST,port);
    let mut conn=TcpStream::connect_timeout(&address.into(), Duration::from_secs(10)).map_err(|_|"Нет связи со Скринмейтом. Убедитесь, что приложение запущено.")?;
    conn.set_read_timeout(Some(Duration::from_secs(15))).map_err(|e|e.to_string())?;
    conn.set_write_timeout(Some(Duration::from_secs(15))).map_err(|e|e.to_string())?;
    let body=json!({"method":method,"params":params}).to_string();
    write!(conn,"POST /rpc HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).map_err(|_|"Ошибка отправки запроса.")?;
    let (status_ok,data)=http_response(&mut io::BufReader::new(conn),MAX_HTTP_BODY)?;
    let response:Value=serde_json::from_slice(&data).map_err(|_|"Некорректный ответ приложения.")?;
    if !status_ok || response["ok"]!=true { return Err(response["error"].as_str().unwrap_or("Ошибка приложения.").to_string()); }
    Ok(response["result"].clone())
}
fn text(s: impl Into<String>) -> Value { json!({"type":"text","text":s.into()}) }
fn frame_contents(frame:&Value) -> Result<Vec<Value>,String> {
    let data=frame["image_base64"].as_str().filter(|s|!s.is_empty()).ok_or("Некорректный кадр.")?;
    if frame["mime_type"]!="image/jpeg" {return Err("Некорректный формат кадра.".into());}
    let w=frame["width"].as_u64().ok_or("Некорректная ширина кадра.")?;
    let h=frame["height"].as_u64().ok_or("Некорректная высота кадра.")?;
    let changed=frame["changed"].as_bool().ok_or("Нет признака изменения кадра.")?;
    Ok(vec![text(format!("Время: {}; размер: {}×{}; экран {}.",frame["timestamp"].as_str().unwrap_or("?"),w,h,if changed {"изменился"} else {"не изменился"})),json!({"type":"image","data":data,"mimeType":"image/jpeg"})])
}
fn validate(name:&str,args:&Value) -> Result<(),String> {
    let props=args.as_object().ok_or("Ожидается объект аргументов.")?;
    let allowed:&[&str]=match name {"look"=>&["monitor","region"],"watch"=>&["seconds","interval"],"say"=>&["text"],"look_diff"|"list_windows"|"inbox"=>&[],_=>return Err("Неизвестный инструмент.".into())};
    if props.keys().any(|k|!allowed.contains(&k.as_str())) {return Err("Неизвестный аргумент.".into());}
    if name=="look" {
        if let Some(m)=props.get("monitor") { if !m.as_str().is_some_and(|s|!s.is_empty()) {return Err("monitor должен быть строкой.".into());} }
        if let Some(region)=props.get("region") {
            let r=region.as_object().ok_or("Некорректная область.")?;
            if r.len()!=4 || ["x","y","width","height"].iter().any(|k|r.get(*k).and_then(Value::as_i64).is_none()) || r["width"].as_i64().unwrap()<=0 || r["height"].as_i64().unwrap()<=0 {return Err("Некорректная область.".into());}
        }
    }
    if name=="say" && !args["text"].as_str().is_some_and(|s|!s.is_empty() && s.chars().count()<=20000) {return Err("Текст должен содержать от 1 до 20000 символов.".into());}
    if name=="watch" { let a=args["seconds"].as_f64().ok_or("Неверный seconds.")?; let b=args["interval"].as_f64().ok_or("Неверный interval.")?; if !(0.0..=60.0).contains(&a) || !(0.2..=60.0).contains(&b) {return Err("Укажите seconds от 0 до 60 и interval от 0.2 до 60.".into());} }
    Ok(())
}
fn call(name:&str,args:&Value) -> Result<Value,String> {
    validate(name,args)?;
    match name {
        "look"=>Ok(json!({"content":frame_contents(&rpc("look",args)?)?})),
        "look_diff"=>{let frame=rpc("look",&json!({}))?; if frame["changed"]==false {Ok(json!({"content":[text("без изменений")]}))} else {Ok(json!({"content":frame_contents(&frame)?}))}},
        "watch"=>{
            let seconds=args["seconds"].as_f64().unwrap(); let interval=args["interval"].as_f64().unwrap();
            let requested=(seconds/interval).floor() as usize + 1; let count=requested.min(5); let mut changes=Vec::new(); let mut frame=Value::Null;
            for i in 0..count { if i>0 {let delay=if requested>5 && i==count-1 {seconds-interval*(i-1) as f64} else {interval}; thread::sleep(Duration::from_secs_f64(delay));} frame=rpc("look",&json!({}))?; if frame["changed"]==true {changes.push(frame["timestamp"].as_str().unwrap_or("?").to_owned());} }
            let mut content=vec![text(format!("Кадров: {count}. Изменения: {}.",if changes.is_empty(){"нет".to_owned()}else{changes.join(", ")}))]; content.extend(frame_contents(&frame)?); Ok(json!({"content":content}))
        },
        _=>Ok(json!({"content":[text(serde_json::to_string_pretty(&rpc(name,args)?).map_err(|e|e.to_string())?)]}))
    }
}
fn handle(request:&Value) -> Value {
    let method=request["method"].as_str().unwrap_or("");
    match method {
        "initialize"=>{let requested=request["params"]["protocolVersion"].as_str().unwrap_or(""); let version=if ["2024-11-05","2025-03-26","2025-06-18","2025-11-25"].contains(&requested) {requested} else {"2025-11-25"}; json!({"result":{"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"screenmate","version":"0.1.0"}}})},
        "ping"=>json!({"result":{}}),
        "tools/list"=>json!({"result":{"tools":schema()}}),
        "tools/call"=>{let name=request["params"]["name"].as_str().unwrap_or(""); let args=request["params"].get("arguments").cloned().unwrap_or(json!({})); match call(name,&args){Ok(result)=>json!({"result":result}),Err(error)=>json!({"result":{"content":[text(error)],"isError":true}})}},
        _=>json!({"error":{"code":-32601,"message":"Method not found"}})
    }
}
fn send(value: Value) {
    let stdout=io::stdout(); let mut out=stdout.lock();
    let _=writeln!(out,"{}",value); let _=out.flush();
}
fn reply(request:&Value)->Option<Value> {
    if request.get("id").is_none() {return None;}
    let response=if request["jsonrpc"]!="2.0" || !request["method"].is_string(){json!({"error":{"code":-32600,"message":"Invalid Request"}})}else{handle(request)};
    let mut response=response.as_object().cloned().unwrap();response.insert("jsonrpc".into(),json!("2.0"));response.insert("id".into(),request["id"].clone());
    Some(Value::Object(response))
}
fn standalone_stdio() {
    let stdin=io::stdin();
    let mut reader=io::BufReader::new(stdin.lock());
    let mut pending:Vec<thread::JoinHandle<()>>=Vec::new();
    loop {
        let line=match daemon::bounded_line(&mut reader,1024*1024) {
            Ok(Some(line))=>line,
            Ok(None)=>break,
            Err(_)=>{send(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid or oversized message"}}));break;}
        };
        let request:Value=match serde_json::from_slice(&line) { Ok(v)=>v, Err(_)=>{send(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}})); continue;} };
        if request.get("id").is_none() {continue;}
        pending.retain(|worker|!worker.is_finished());
        if pending.len()>=16 {send(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"Too many requests"}}));continue;}
        // A long watch must not block ping or independent requests.
        pending.push(thread::spawn(move || {
            if let Some(response)=reply(&request) {send(response);}
        }));
    }
    for worker in pending {let _=worker.join();}
}
fn main() {
    let args:Vec<_>=env::args_os().skip(1).collect();
    if args.iter().any(|arg|arg=="--stop") {
        if let Err(error)=daemon::stop() {eprintln!("Screenmate MCP: {error}");std::process::exit(1);}
    } else if args.iter().any(|arg|arg=="--daemon") {
        if let Err(error)=daemon::run() {eprintln!("Screenmate MCP: {error}");std::process::exit(1);}
    } else if let Some(stream)=daemon::connect() {
        if let Err(error)=daemon::bridge(stream) {eprintln!("Screenmate MCP: {error}");std::process::exit(1);}
    } else {
        // Auto-start may be disabled or the daemon may have crashed. Preserve
        // the original, fully functional local stdio server in that case.
        standalone_stdio();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test] fn chunked_response_handles_multiple_chunks_extensions_and_trailers() {
        let wire=b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;foo=bar\r\nWiki\r\n5\r\npedia\r\n0\r\nX-Finished: yes\r\n\r\n";
        let (ok,body)=http_response(&mut Cursor::new(wire),9).unwrap();
        assert!(ok);assert_eq!(body,b"Wikipedia");
    }
    #[test] fn content_length_and_http_errors_remain_supported() {
        let (ok,body)=http_response(&mut Cursor::new(b"HTTP/1.0 401 Unauthorized\r\nContent-Length: 4\r\n\r\nnope"),4).unwrap();
        assert!(!ok);assert_eq!(body,b"nope");
        assert!(http_response(&mut Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"),4).is_err());
    }
    #[test] fn rejects_malformed_truncated_ambiguous_and_oversized_chunks() {
        let cases:&[&[u8]]=&[
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\ng\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nab",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na!!0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\n\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nmissing-colon\r\n\r\n",
        ];
        for wire in cases {assert!(http_response(&mut Cursor::new(wire),4).is_err(),"accepted malformed response: {wire:?}");}
        let mut long_line=b"HTTP/1.1 200 OK\r\nX-Test: ".to_vec();long_line.extend(vec![b'x';MAX_HTTP_LINE]);
        assert!(http_response(&mut Cursor::new(long_line),4).is_err());
    }
    #[test] fn metadata_and_image() {
        let frame=json!({"image_base64":"/9j/2Q==","mime_type":"image/jpeg","timestamp":"now","width":4,"height":2,"changed":true});
        let items=frame_contents(&frame).unwrap(); assert!(items[0]["text"].as_str().unwrap().contains("4×2")); assert_eq!(items[1]["type"],"image");
        assert!(frame_contents(&json!({})).is_err());
    }
    #[test] fn validation_rejects_unexpected_args_and_bad_bounds() {
        assert!(validate("watch",&json!({"seconds":60,"interval":0})).is_err());
        assert!(validate("look",&json!({"region":{"x":0,"y":0,"width":-1,"height":1}})).is_err());
        assert!(validate("look",&json!({"monitor":"1"})).is_ok());
        assert!(validate("say",&json!({"text":"hello","unknown":true})).is_err());
    }
    #[test] fn initialize_and_list() {
        let answer=handle(&json!({"method":"initialize","params":{"protocolVersion":"2025-06-18"}}));
        assert_eq!(answer["result"]["protocolVersion"],"2025-06-18");
        assert_eq!(handle(&json!({"method":"tools/list"}))["result"]["tools"].as_array().unwrap().len(),6);
    }
}
