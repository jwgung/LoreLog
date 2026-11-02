// The addresses other devices may put in, shared by the first-run guide and the settings: this PC's tailnet address
// (first choice) and, when the user turned on "같은 와이파이에서도 받기" (off by default), its addresses in the same
// Wi-Fi. Everything goes into the page as text.
export const LAN_NOTE =
  "같은 와이파이 안에서만 닿고, 기록과 토큰이 암호화 없이 와이파이로 갑니다. 집이나 사무실처럼 믿는 와이파이에서만 쓰세요.";

// Fill the <select> with the offered addresses and return them ([] when there is none).
export function fillAddresses(select, info) {
  const offered = [];
  if (info && info.address_for_devices) offered.push({ url: info.address_for_devices, lan: false });
  for (const url of (info && info.lan_addresses) || []) offered.push({ url, lan: true });
  select.replaceChildren(...offered.map(({ url, lan }) => {
    const o = document.createElement("option");
    o.value = url;
    o.textContent = (lan ? "같은 와이파이 · " : "Tailscale · ") + url;
    o.dataset.lan = lan ? "1" : "";
    return o;
  }));
  select.disabled = !offered.length;
  return offered;
}

// What to say when no address is offered yet.
export function noAddressNote(info) {
  return info && info.same_wifi
    ? "아직 다른 기기가 닿을 주소가 없습니다. 이 PC를 와이파이(공유기)에 잇거나, 이 PC와 폰에 Tailscale을 켜고 로그인하면 몇십 초 뒤 생깁니다. 그 뒤 '다시 보기'."
    : "아직 다른 기기가 닿을 주소가 없습니다. 이 PC와 폰에 Tailscale을 켜고 로그인하면 몇십 초 뒤 생깁니다. 같은 와이파이 안에서만 쓰려면 아래 '같은 와이파이에서도 받기'를 켜세요.";
}

// Is the chosen address one in the same Wi-Fi (plain http, not encrypted)?
export function chosenIsLan(select) {
  const o = select.selectedOptions[0];
  return !!(o && o.dataset.lan);
}

// The "같은 와이파이에서도 받기" checkbox: shows the setting, saves a change (the shell starts or stops listening at
// once) and calls `changed` with the new info. On failure the box goes back and `failed` gets the error.
export function wireSameWifi(box, invoke, changed, failed) {
  box.onchange = async () => {
    const on = box.checked;
    box.disabled = true;
    try { changed(await invoke("same_wifi_set", { on })); }
    catch (e) { box.checked = !on; failed(String(e)); }
    box.disabled = false;
  };
}
