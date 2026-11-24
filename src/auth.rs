//! Spotify OAuth (Authorization Code + PKCE). No client secret is needed or stored.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const REDIRECT: &str = "http://127.0.0.1:8888/callback";
const SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing user-library-read playlist-read-private playlist-read-collaborative user-read-recently-played user-top-read user-follow-read";

#[derive(Clone)]
struct Token {
    access: String,
    refresh: String,
    expires_at: u64,
}

pub struct Auth {
    client_id: String,
    tok: Mutex<Option<Token>>,
    http: reqwest::blocking::Client,
}

pub fn config_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/rola")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random_string(n: usize) -> String {
    let mut buf = vec![0u8; n];
    fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).expect("cannot read /dev/urandom");
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    buf.iter().map(|b| A[*b as usize % A.len()] as char).collect()
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

impl Auth {
    pub fn new() -> Result<Auth, String> {
        let client_id = std::env::var("ROLA_CLIENT_ID")
            .ok()
            .or_else(|| fs::read_to_string(config_dir().join("client_id")).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or("no Spotify Client ID: put it in ~/.config/rola/client_id (or set ROLA_CLIENT_ID)")?;
        let tok = fs::read_to_string(config_dir().join("auth.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| {
            Some(Token { access: v["access"].as_str()?.into(), refresh: v["refresh"].as_str()?.into(), expires_at: v["expires_at"].as_u64()? })
        });
        let http = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
        Ok(Auth { client_id, tok: Mutex::new(tok), http })
    }

    pub fn logged_in(&self) -> bool {
        self.tok.lock().unwrap().is_some()
    }

    fn save(&self, t: &Token) {
        let _ = fs::create_dir_all(config_dir());
        let body = serde_json::json!({"access": t.access, "refresh": t.refresh, "expires_at": t.expires_at}).to_string();
        if let Ok(mut f) = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(config_dir().join("auth.json")) {
            let _ = f.write_all(body.as_bytes());
        }
    }

    fn store(&self, v: &Value, old_refresh: Option<&str>) -> Result<(), String> {
        let access = v["access_token"].as_str().ok_or_else(|| format!("token response had no access_token: {v}"))?;
        let refresh = v["refresh_token"].as_str().or(old_refresh).ok_or("token response had no refresh_token")?;
        let t = Token { access: access.into(), refresh: refresh.into(), expires_at: now() + v["expires_in"].as_u64().unwrap_or(3600) };
        self.save(&t);
        *self.tok.lock().unwrap() = Some(t);
        Ok(())
    }

    fn token_request(&self, form: &[(&str, &str)]) -> Result<Value, String> {
        let r = self.http.post("https://accounts.spotify.com/api/token").form(form).send().map_err(|e| e.to_string())?;
        let ok = r.status().is_success();
        let v: Value = r.json().unwrap_or(Value::Null);
        if ok { Ok(v) } else { Err(format!("token endpoint: {}", v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("request failed"))) }
    }

    /// Browser login. Run before the TUI starts.
    pub fn login(&self) -> Result<(), String> {
        let verifier = random_string(64);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = random_string(16);
        let url = format!(
            "https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&state={state}&code_challenge_method=S256&code_challenge={challenge}",
            self.client_id,
            enc(SCOPES),
            enc(REDIRECT)
        );
        let listener = TcpListener::bind("127.0.0.1:8888").map_err(|e| format!("cannot listen on 127.0.0.1:8888 ({e}) — is something else using that port?"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        println!("Opening your browser to log in to Spotify.\nIf nothing opens, visit:\n{url}\n");
        let _ = open::that(&url);

        let deadline = Instant::now() + Duration::from_secs(240);
        let query_of = |req: &str| req.lines().next().unwrap_or("").split_whitespace().nth(1).and_then(|p| p.split_once('?')).map(|x| x.1.to_string()).unwrap_or_default();
        let param = |query: &str, k: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(a, _)| *a == k).map(|x| x.1.to_string()));
        let query = loop {
            match listener.accept() {
                Ok((mut s, _)) => {
                    let _ = s.set_nonblocking(false);
                    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut buf = [0u8; 4096];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let q = query_of(&req);
                    let reply = |s: &mut std::net::TcpStream, status: &str, msg: &str| {
                        let page = format!("<html><body style='font-family:sans-serif;background:#111;color:#eee;padding:3em'><h2>{msg}</h2></body></html>");
                        let _ = s.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes());
                    };
                    if !req.starts_with("GET /callback") {
                        reply(&mut s, "404 Not Found", "not found");
                    } else if param(&q, "state").as_deref() != Some(state.as_str()) {
                        // Stale tab from an earlier attempt: ignore it and keep waiting for the real one.
                        reply(&mut s, "400 Bad Request", "Stale login tab — use the newest rola login window.");
                    } else {
                        reply(&mut s, "200 OK", "rola is logged in. You can close this tab.");
                        break q;
                    }
                }
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => return Err("login timed out".into()),
            }
        };
        let get = |k: &str| param(&query, k);
        if let Some(e) = get("error") {
            return Err(format!("Spotify refused the login: {e}"));
        }
        let code = get("code").ok_or("no code in callback")?;
        let v = self.token_request(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", REDIRECT),
            ("client_id", &self.client_id),
            ("code_verifier", &verifier),
        ])?;
        self.store(&v, None)
    }

    pub fn invalidate(&self) {
        if let Some(t) = self.tok.lock().unwrap().as_mut() {
            t.expires_at = 0;
        }
    }

    /// A valid access token, refreshing if it is about to expire.
    pub fn token(&self) -> Result<String, String> {
        let cur = self.tok.lock().unwrap().clone().ok_or("not logged in — run `rola login`")?;
        if cur.expires_at > now() + 60 {
            return Ok(cur.access);
        }
        let v = self.token_request(&[("grant_type", "refresh_token"), ("refresh_token", &cur.refresh), ("client_id", &self.client_id)])?;
        self.store(&v, Some(&cur.refresh))?;
        Ok(self.tok.lock().unwrap().as_ref().unwrap().access.clone())
    }
}
