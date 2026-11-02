// The addresses other devices may put in, shared by the first-run guide and the settings: this PC's tailnet address
// (first choice) and its addresses in the same Wi-Fi. Everything goes into the page as text.
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

// Is the chosen address one in the same Wi-Fi (plain http, not encrypted)?
export function chosenIsLan(select) {
  const o = select.selectedOptions[0];
  return !!(o && o.dataset.lan);
}
