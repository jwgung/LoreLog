"use strict";
// The schedule (#62): the events the vault holds from today for seven days, a week back or ahead at a time, each with
// the assistant's notes as the server writes them (templates; guesses say 추정). Clicking an event opens the
// assistant with a question about it (index.html?ask=…). Every value from the server goes into the page as text.
const $ = (id) => document.getElementById(id);
const invoke = window.__TAURI__?.core?.invoke;
let API_BASE = "", token = "";
const DAYS = 7;
let offset = 0;   // weeks from today

function el(tag, text, cls) {
  const e = document.createElement(tag);
  if (text !== undefined) e.textContent = text;
  if (cls) e.className = cls;
  return e;
}
const say = (t, bad) => { $("smsg").textContent = t; $("smsg").className = "smsg" + (bad ? " error" : ""); };

const ymd = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
function addDays(d, n) { const x = new Date(d); x.setDate(x.getDate() + n); return x; }
function startOfToday() { const d = new Date(); d.setHours(0, 0, 0, 0); return d; }

function dayLabel(day, today) {
  const d = new Date(day + "T00:00");
  const opts = { month: "long", day: "numeric", weekday: "long", ...(d.getFullYear() !== today.getFullYear() ? { year: "numeric" } : {}) };
  const s = d.toLocaleDateString("ko-KR", opts);
  if (day === ymd(today)) return "오늘 · " + s;
  if (day === ymd(addDays(today, 1))) return "내일 · " + s;
  return s;
}

function time(e) {
  if (e.all_day || !e.start) return "하루 종일";
  return e.end ? `${e.start}~${e.end}` : e.start;
}

function eventButton(e) {
  const b = el("button", undefined, "sevent");
  b.type = "button";
  b.append(el("div", e.title || "(제목 없음)", "stitle"));
  const meta = [time(e), e.place, (e.attendees || []).slice(0, 3).join(", ")].filter((x) => x);
  b.append(el("div", meta.join(" · "), "smeta"));
  for (const n of e.notes || []) {
    b.append(el("div", n.text, "snote" + (n.kind === "request" ? " request" : n.guess ? " guess" : "")));
  }
  b.title = "대화로 묻기";
  b.onclick = () => { if (e.question) location.href = "index.html?ask=" + encodeURIComponent(e.question); };
  return b;
}

async function load() {
  const today = startOfToday();
  const from = addDays(today, DAYS * offset), to = addDays(from, DAYS - 1);
  $("span").textContent = `${from.getMonth() + 1}월 ${from.getDate()}일 ~ ${to.getMonth() + 1}월 ${to.getDate()}일`;
  $("today").hidden = offset === 0;
  const days = $("days");
  days.replaceChildren();
  if (!API_BASE || !token) { say("서버에 연결한 뒤에 볼 수 있습니다. 설정에서 연결해 주세요.", true); return; }
  say("불러오는 중…");
  const res = await fetch(`${API_BASE}/schedule?from=${ymd(from)}&to=${ymd(to)}`, { headers: { Authorization: `Bearer ${token}` } });
  let data = {};
  try { data = await res.json(); } catch { /* empty */ }
  if (res.status === 503) { say("금고가 아직 기록을 정리하지 않았어요. 잠시 뒤 다시 열어 주세요."); return; }
  if (res.status !== 200) { say("불러오지 못했습니다: " + (data.error || res.status), true); return; }
  const items = Array.isArray(data.items) ? data.items : [];
  say(items.length ? "" : "이 주에는 금고에 들어온 일정이 없어요.");
  for (let i = 0; i < DAYS; i++) {
    const day = ymd(addDays(from, i));
    days.append(el("div", dayLabel(day, today), "sday"));
    const mine = items.filter((e) => e.day === day)
      .sort((a, b) => (b.all_day - a.all_day) || String(a.start).localeCompare(String(b.start)));
    if (!mine.length) days.append(el("div", "일정 없음", "snone"));
    for (const e of mine) days.append(eventButton(e));
  }
}

const go = (n) => { offset = n; load().catch((e) => say(String(e), true)); };
$("prev").addEventListener("click", () => go(offset - 1));
$("next").addEventListener("click", () => go(offset + 1));
$("today").addEventListener("click", () => go(0));

(async () => {
  try {
    const s = await invoke("server_config");
    API_BASE = (s.base || "").replace(/\/+$/, "");
    token = s.token || "";
  } catch (e) { say(String(e), true); return; }
  go(0);
})();
