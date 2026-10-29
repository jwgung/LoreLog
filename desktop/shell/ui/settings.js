// The settings page. It talks to the Rust shell through tauri invoke; the shell owns the config file.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const msg = (text, ok) => { const m = $("msg"); m.textContent = text; m.className = "msg " + (ok ? "ok" : "err"); };

async function load() {
  try {
    const s = await invoke("get_settings");
    $("url").value = s.url || "";
    $("token").placeholder = s.has_token ? "저장됨 — 바꾸려면 새로 입력" : "서버의 INGEST_TOKEN";
    $("cfgpath").textContent = "설정 파일: " + s.config_path;
    renderWatch(s.watch_dirs || []);
  } catch (e) { msg("설정을 읽지 못했습니다: " + e, false); }
}

function renderWatch(dirs) {
  const ul = $("watchlist");
  ul.innerHTML = "";
  if (dirs.length === 0) { ul.innerHTML = '<li style="opacity:.6;border:0;">아직 없음</li>'; return; }
  for (const d of dirs) {
    const li = document.createElement("li");
    const span = document.createElement("span"); span.textContent = d;
    const b = document.createElement("button"); b.textContent = "빼기";
    b.onclick = async () => { try { renderWatch(await invoke("remove_watch", { dir: d })); } catch (e) { msg(String(e), false); } };
    li.append(span, b); ul.appendChild(li);
  }
}

$("save").onclick = async () => {
  try {
    await invoke("save_settings", { url: $("url").value, token: $("token").value || null });
    location.href = "index.html";   // back to the assistant, which now reads the saved config
  } catch (e) { msg(String(e), false); }
};
$("addwatch").onclick = async () => {
  const d = $("watchdir").value.trim();
  if (!d) return;
  try { renderWatch(await invoke("add_watch", { dir: d })); $("watchdir").value = ""; msg("더했습니다.", true); }
  catch (e) { msg(String(e), false); }
};

load();
