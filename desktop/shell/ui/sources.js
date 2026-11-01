// The mail, calendar and address book forms, shared by the settings page and the first-run guide (welcome.html). Both
// pages carry the same element ids; a page without the address book part simply has no b* elements. Every value from
// the shell goes into the page as text (textContent / value).
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const say = (id, text, ok) => { const m = $(id); m.textContent = text; m.className = "msg " + (ok ? "ok" : "err"); };
const el = (tag, text) => { const e = document.createElement(tag); if (text !== undefined) e.textContent = text; return e; };

function checks(ul, items) {
  ul.replaceChildren();
  for (const it of items) {
    const li = el("li");
    const label = el("label");
    const box = el("input");
    box.type = "checkbox";
    box.checked = it.chosen;
    box.value = it.value;
    label.append(box, document.createTextNode(it.label));
    li.append(label);
    ul.append(li);
  }
}
const checked = (ul) => [...ul.querySelectorAll("input:checked")].map((b) => b.value);
const serverOf = (u) => { try { const p = new URL(u); return `${p.protocol}//${p.host}/`; } catch { return u; } };

// What is collected now, into the forms.
export async function loadSources() {
  const v = await invoke("sources_view").catch(() => null);
  if (!v) return;
  if (v.mail) {
    $("mHost").value = v.mail.host;
    $("mPort").value = String(v.mail.port);
    $("mUser").value = v.mail.user;
    $("mPass").placeholder = v.mail.has_password ? "저장됨 — 바꾸려면 새로 입력" : "";
    say("mMsg", "모으는 폴더: " + v.mail.folder_labels.join(", "), true);
  }
  if ($("bServer") && v.books && v.books.length) {
    $("bServer").value = serverOf(v.books[0]);
    say("bMsg", `주소록 ${v.books.length}개를 모읍니다.`, true);
    $("bOff").hidden = false;
  }
  if (v.calendars.length) {
    $("cServer").value = serverOf(v.calendars[0]);
    say("cMsg", `달력 ${v.calendars.length}개를 모읍니다.`, true);
    $("cOff").hidden = false;
  }
}

// Wire the forms. `onSaved` runs after a source was saved (the shell then syncs).
export function initSources(onSaved = () => {}) {
  const mailForm = () => ({
    host: $("mHost").value.trim(),
    port: Number($("mPort").value.trim() || "993"),
    user: $("mUser").value.trim(),
    password: $("mPass").value || null,
  });

  $("mConnect").onclick = async () => {
    const f = mailForm();
    if (!Number.isInteger(f.port) || f.port <= 0 || f.port > 65535) { say("mMsg", "포트는 숫자입니다.", false); return; }
    say("mMsg", "연결하는 중…", true);
    try {
      const folders = await invoke("mail_folders", f);
      checks($("mFolders"), folders.map((x) => ({ value: x.name, label: x.label, chosen: x.chosen })));
      $("mSave").hidden = false;
      say("mMsg", `연결했습니다. 폴더 ${folders.length}개 — 모을 폴더를 고르고 저장하세요.`, true);
    } catch (e) { say("mMsg", String(e), false); }
  };
  // the folders shown belong to the server and account they were read from: change those and connect again
  for (const id of ["mHost", "mPort", "mUser", "mPass"]) {
    $(id).addEventListener("input", () => { $("mSave").hidden = true; $("mFolders").replaceChildren(); });
  }
  $("mSave").onclick = async () => {
    try {
      await invoke("mail_save", { ...mailForm(), folders: checked($("mFolders")) });
      $("mPass").value = "";
      $("mPass").placeholder = "저장됨 — 바꾸려면 새로 입력";
      say("mMsg", "저장했습니다. 지금 메일을 가져옵니다.", true);
      onSaved();
    } catch (e) { say("mMsg", String(e), false); }
  };

  $("cFind").onclick = async () => {
    say("cMsg", "달력을 찾는 중…", true);
    try {
      const cals = await invoke("calendar_list", { server: $("cServer").value.trim() });
      checks($("cList"), cals.map((c) => ({ value: c.url, label: `${c.name} (${c.components})`, chosen: c.chosen })));
      $("cSave").hidden = false;
      say("cMsg", `달력 ${cals.length}개 — 모을 달력을 고르고 저장하세요.`, true);
    } catch (e) { say("cMsg", String(e), false); }
  };
  $("cSave").onclick = async () => {
    const urls = checked($("cList"));
    if (!urls.length) { say("cMsg", "달력을 하나 이상 고르세요. 그만 모으려면 '일정 모으지 않기'.", false); return; }
    try { await invoke("calendar_save", { urls }); say("cMsg", "저장했습니다. 지금 일정을 가져옵니다.", true); $("cOff").hidden = false; onSaved(); }
    catch (e) { say("cMsg", String(e), false); }
  };
  $("cOff").onclick = async () => {
    try { await invoke("calendar_save", { urls: [] }); $("cList").replaceChildren(); $("cSave").hidden = true; $("cOff").hidden = true; say("cMsg", "일정을 모으지 않습니다. 이미 만든 노트는 그대로 둡니다.", true); }
    catch (e) { say("cMsg", String(e), false); }
  };

  if (!$("bFind")) return;
  $("bFind").onclick = async () => {
    say("bMsg", "주소록을 찾는 중…", true);
    try {
      const books = await invoke("book_list", { server: $("bServer").value.trim() });
      checks($("bList"), books.map((b) => ({ value: b.url, label: b.name, chosen: b.chosen })));
      $("bSave").hidden = false;
      say("bMsg", `주소록 ${books.length}개 — 모을 주소록을 고르고 저장하세요.`, true);
    } catch (e) { say("bMsg", String(e), false); }
  };
  $("bSave").onclick = async () => {
    const urls = checked($("bList"));
    if (!urls.length) { say("bMsg", "주소록을 하나 이상 고르세요. 그만 모으려면 '주소록 모으지 않기'.", false); return; }
    try { await invoke("book_save", { urls }); say("bMsg", "저장했습니다. 지금 주소록을 가져옵니다.", true); $("bOff").hidden = false; onSaved(); }
    catch (e) { say("bMsg", String(e), false); }
  };
  $("bOff").onclick = async () => {
    try { await invoke("book_save", { urls: [] }); $("bList").replaceChildren(); $("bSave").hidden = true; $("bOff").hidden = true; say("bMsg", "주소록을 모으지 않습니다.", true); }
    catch (e) { say("bMsg", String(e), false); }
  };
}
