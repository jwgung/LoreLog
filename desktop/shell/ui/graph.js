"use strict";
// The graph screen (#47): find a node by any of its names, see its relations with where each came from, and correct
// them. Corrections go to the server as the same {wrong, right, ref} pair as any correction (#10). Every value from
// the server goes into the page as text (textContent / value).
const $ = (id) => document.getElementById(id);
const invoke = window.__TAURI__?.core?.invoke;
let API_BASE = "", token = "";

const KIND = { person: "사람", org: "조직", case: "건", doc: "문서", thing: "대상", todo: "할 일", event: "일정" };
const REL = { member: "소속", role: "역할", state: "상태", sent: "보냄", requested: "요청함", todo: "할 일", about: "관련", uses: "사용" };

function el(tag, text, cls) {
  const e = document.createElement(tag);
  if (text !== undefined) e.textContent = text;
  if (cls) e.className = cls;
  return e;
}
const say = (t, bad) => { $("gmsg").textContent = t; $("gmsg").className = "gmsg" + (bad ? " error" : ""); };

async function api(path, body) {
  const res = await fetch(API_BASE + path, {
    method: body === undefined ? "GET" : "POST",
    headers: { Authorization: `Bearer ${token}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  let data = {};
  try { data = await res.json(); } catch { /* empty */ }
  return { status: res.status, data };
}

async function search(q) {
  const ul = $("gfound");
  ul.replaceChildren();
  if (q.trim().length < 2) return;
  const r = await api("/graph/search?q=" + encodeURIComponent(q.trim()));
  if (r.status !== 200) { say("찾지 못했습니다: " + (r.data.error || r.status), true); return; }
  if (!r.data.nodes.length) { say("그 이름으로 아는 것이 없습니다."); return; }
  say("");
  for (const n of r.data.nodes) {
    const b = el("button");
    b.append(el("span", n.name || n.id), el("span", `${KIND[n.kind] || n.kind} · 관계 ${n.edges}개`, "gkind"));
    if (n.aliases.length) b.append(el("span", " · " + n.aliases.join(", "), "gkind"));
    b.onclick = () => show(n.id);
    const li = el("li"); li.append(b); ul.append(li);
  }
  if (r.data.nodes.length === 1) show(r.data.nodes[0].id);
}

async function correct(body, then) {
  const r = await api("/corrections", body);
  if (r.status === 200) { say(r.data.changed || "고쳤습니다."); await then(); }
  else say("고치지 못했습니다: " + (r.data.error || r.status), true);
}

async function show(id) {
  const r = await api("/graph/node?id=" + encodeURIComponent(id));
  const view = $("gview");
  view.replaceChildren();
  if (r.status !== 200) { say("열지 못했습니다: " + (r.data.error || r.status), true); return; }
  const { node, same, edges } = r.data;
  const box = el("div", undefined, "gnode");
  box.append(el("h2", node.name || node.id));
  const names = [node.id, ...node.aliases].concat(node.short.length ? ["(줄임말 추측: " + node.short.join(", ") + ")"] : []);
  box.append(el("div", `${KIND[node.kind] || node.kind} · ${names.join(" · ")}`, "gnames"));
  if (same.length) box.append(el("div", "같은 조직: " + same.join(", "), "gnames"));
  if (node.kind === "org" && !node.id.startsWith("org:~")) {
    const domain = node.id.slice(4);
    const row = el("div", undefined, "grow");
    const name = el("input"); name.placeholder = "내가 부르는 이름";
    const addName = el("button", "이름 더하기", "gbtn");
    addName.onclick = () => name.value.trim() &&
      correct({ ref: "org:" + domain, wrong: node.name || domain, right: name.value.trim() }, () => show(id));
    const other = el("input"); other.placeholder = "같은 조직의 다른 도메인";
    const link = el("button", "같은 조직으로 잇기", "gbtn");
    link.onclick = () => other.value.trim() &&
      correct({ ref: "org:" + domain, wrong: domain, right: "org:" + other.value.trim().replace(/^org:/, "") }, () => show(id));
    row.append(name, addName, other, link);
    box.append(row);
  }
  const table = el("table", undefined, "gedges");
  const head = el("tr");
  for (const h of ["관계", "상대", "이름·상태", "날짜", "근거", ""]) head.append(el("th", h));
  table.append(head);
  for (const e of edges) {
    const tr = el("tr");
    tr.append(el("td", (REL[e.rel] || e.rel) + (e.dir === "in" ? " ←" : " →")));
    const other = el("td");
    if (e.other) { const a = el("a", e.other_name); a.onclick = () => show(e.other); other.append(a); }
    tr.append(other, el("td", e.label), el("td", e.since));
    const ev = el("td");
    ev.append(el("div", e.evidence ? `«${e.evidence}»` : ""), el("div", `${e.day} ${e.title}`, "gev"));
    tr.append(ev);
    const act = el("td");
    if (e.origin === "model") {
      const fix = el("button", "고치기", "gbtn small");
      fix.onclick = () => {
        const v = prompt("맞는 이름이나 상태를 적어 주세요", e.label);
        if (v && v.trim() && v.trim() !== e.label) correct({ ref: "edge:" + e.key, wrong: e.label || e.rel, right: v.trim() }, () => show(id));
      };
      const drop = el("button", "틀림", "gbtn small");
      drop.onclick = () => confirm("이 관계를 지울까요? 근거 기록은 그대로 남습니다.") &&
        correct({ ref: "edge:" + e.key, wrong: `${REL[e.rel] || e.rel} ${e.other_name} ${e.label}`.trim(), right: "-" }, () => show(id));
      act.append(fix, drop);
    } else {
      act.append(el("span", "메일 머리말", "gev"));
    }
    tr.append(act);
    table.append(tr);
  }
  if (!edges.length) box.append(el("p", "아직 관계가 없습니다. 기록을 읽는 중일 수 있어요.", "muted"));
  else box.append(table);
  view.append(box);
}

$("gform").addEventListener("submit", (ev) => { ev.preventDefault(); search($("gq").value).catch((e) => say(String(e), true)); });

(async () => {
  try {
    const s = await invoke("server_config");
    API_BASE = (s.base || "").replace(/\/+$/, "");
    token = s.token || "";
    if (!API_BASE || !token) say("서버에 연결한 뒤에 볼 수 있습니다. 설정에서 연결해 주세요.", true);
  } catch (e) { say(String(e), true); }
})();
