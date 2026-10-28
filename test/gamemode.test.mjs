// Functional smoke test for GameMode (hold/toggle/turbo/release) using the
// compiled www/static/lib.js and a minimal DOM stub. Not part of the build:
// run `tsc` first, then `node test/gamemode.test.mjs`.
import fs from "node:fs";

async function main() {

const src = fs.readFileSync("www/static/lib.js", "utf8");
const chunk = src.slice(src.indexOf("const GAME_PRESETS"), src.indexOf("class StageOverlay"));

function make_el(id) {
    const classes = new Set();
    const listeners = {};
    const children = [];
    const el = {
        id: id ?? "",
        textContent: "",
        title: "",
        type: "",
        className: "",
        maxLength: 0,
        placeholder: "",
        value: "",
        checked: false,
        disabled: false,
        style: { setProperty(k, v) { this[k] = v; } },
        get nextElementSibling() { return this._next ??= make_el(); },
        classList: {
            add: (...cs) => cs.forEach((c) => classes.add(c)),
            remove: (...cs) => cs.forEach((c) => classes.delete(c)),
            toggle: (c, force) => {
                const want = force === undefined ? !classes.has(c) : !!force;
                if (want) classes.add(c); else classes.delete(c);
                return want;
            },
            contains: (c) => classes.has(c),
        },
        setAttribute() { },
        getAttribute() { return null; },
        addEventListener(t, fn) { (listeners[t] ??= []).push(fn); },
        removeEventListener() { },
        setPointerCapture() { },
        releasePointerCapture() { },
        getBoundingClientRect: () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100 }),
        appendChild(c) { children.push(c); return c; },
        remove() { },
        removeAttribute() { },
        querySelectorAll(sel) {
            if (sel === ".game_btn") return children.filter((c) => c.classList.contains("game_btn"));
            if (sel === ".game_btn.selected") return children.filter((c) => c.classList.contains("game_btn") && c.classList.contains("selected"));
            return [];
        },
        fire(type, ev) { for (const fn of listeners[type] ?? []) fn(ev); },
    };
    return el;
}

const by_id = new Map();
for (const id of ["game_overlay", "game_stick_base", "game_stick", "game_preset",
    "game_btn_size", "game_btn_alpha", "game_edit", "game_settings", "handle"])
    by_id.set(id, make_el(id));

const sent = [];
const fake_ws = { readyState: 1, send: (p) => sent.push(JSON.parse(p)) };

const document_stub = {
    createElement: () => make_el(),
    getElementById: (id) => by_id.get(id) ?? null,
    addEventListener() { },
    hidden: false,
    body: make_el("body"),
    fullscreenElement: null,
};
const storage = new Map();
const alerts = [];
let prompt_impl = () => null;
const settings_stub = {
    checks: new Map([["game_mode", by_id.get("game_mode_check")]]),
    game_sensitivity_input: { valueAsNumber: 1 },
    save_settings() { },
};
by_id.set("game_mode_check", Object.assign(make_el("game_mode"), { onchange: null }));
settings_stub.checks.set("game_mode", by_id.get("game_mode_check"));

const sandbox = {
    document: document_stub,
    navigator: { vibrate: () => { } },
    localStorage: {
        getItem: (k) => storage.get(k) ?? null,
        setItem: (k, v) => storage.set(k, v),
    },
    WebSocket: function () { },
    settings: settings_stub,
    log: () => { },
    LogLevel: { DEBUG: 0, WARN: 1, ERROR: 2 },
    wire_audio_button: () => { },
    setInterval, clearInterval, setTimeout,
    prompt: (...a) => prompt_impl(...a),
    alert: (m) => alerts.push(m),
};
sandbox.window = sandbox;
sandbox.globalThis = sandbox;
sandbox.WebSocket.OPEN = 1;

const factory = new Function(
    "document", "navigator", "localStorage", "WebSocket", "settings", "log", "LogLevel",
    "wire_audio_button", "setInterval", "clearInterval", "setTimeout", "prompt", "alert", "navigator",
    chunk + "\n; return { GameMode, GAME_PRESETS, GAME_LAYOUT_KEY };");
const { GameMode, GAME_LAYOUT_KEY } = factory(
    sandbox.document, sandbox.navigator, sandbox.localStorage, sandbox.WebSocket, settings_stub,
    sandbox.log, sandbox.LogLevel, sandbox.wire_audio_button, setInterval, clearInterval,
    setTimeout, sandbox.prompt, sandbox.alert, sandbox.navigator);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const fails = [];
const check = (name, cond) => { console.log((cond ? "PASS " : "FAIL ") + name); if (!cond) fails.push(name); };
const downs = (code) => sent.filter((m) => m.KeyboardEvent && m.KeyboardEvent.code === code && m.KeyboardEvent.event_type === "down").length;
const ups = (code) => sent.filter((m) => m.KeyboardEvent && m.KeyboardEvent.code === code && m.KeyboardEvent.event_type === "up").length;

const gm = new GameMode(fake_ws);
gm.set_enabled(true);
check("overlay shown", !by_id.get("game_overlay").classList.contains("hide"));

// --- hold mode: press while finger down, release on lift
const hold_spec = gm.specs.find((s) => s.id === "gb_reload");
const hold_el = gm.btn_els.get("gb_reload");
hold_el.fire("pointerdown", { pointerId: 1, stopPropagation() { } });
check("hold: key down", downs("KeyR") === 1);
hold_el.fire("pointerup", { pointerId: 1 });
check("hold: key up on lift", ups("KeyR") === 1);

// --- toggle mode: latch on first tap, release on second
gm.specs.find((s) => s.id === "gb_crouch").mode = "toggle";
gm.render_buttons();
const tog_el = gm.btn_els.get("gb_crouch");
tog_el.fire("pointerdown", { pointerId: 2, stopPropagation() { } });
check("toggle: latched down", downs("ControlLeft") === 1 && ups("ControlLeft") === 0);
tog_el.fire("pointerup", { pointerId: 2 });
check("toggle: survives lift", ups("ControlLeft") === 0);
tog_el.fire("pointerdown", { pointerId: 3, stopPropagation() { } });
check("toggle: released by 2nd tap", ups("ControlLeft") === 1);

// --- turbo mode: repeated pulses while held, none after lift
gm.specs.find((s) => s.id === "gb_fire").mode = "turbo";
gm.render_buttons();
const tur_el = gm.btn_els.get("gb_fire");
tur_el.fire("pointerdown", { pointerId: 4, stopPropagation() { } });
await sleep(260);
tur_el.fire("pointerup", { pointerId: 4 });
const click_downs = sent.filter((m) => m.ClickEvent && m.ClickEvent.button === "left" && m.ClickEvent.action === "down").length;
await sleep(100); // the last pulse's "up" lands ~35ms after its "down"
const click_ups = sent.filter((m) => m.ClickEvent && m.ClickEvent.button === "left" && m.ClickEvent.action === "up").length;
check("turbo: fired repeatedly", click_downs >= 3);
check("turbo: pulses balanced", click_downs === click_ups);
await sleep(150);
check("turbo: stops after lift", sent.filter((m) => m.ClickEvent && m.ClickEvent.button === "left" && m.ClickEvent.action === "down").length === click_downs);

// --- release_all lets go of a latched toggle
tog_el.fire("pointerdown", { pointerId: 5, stopPropagation() { } });
check("toggle latched again", downs("ControlLeft") === 2 && gm.toggle_on.has("gb_crouch"));
gm.release_all();
check("release_all: toggle up", ups("ControlLeft") === 2 && gm.toggle_on.size === 0);

// --- stick in edit mode moves and persists; leaving edit releases keys
gm.set_editing(true);
check("edit mode class", by_id.get("game_overlay").classList.contains("edit"));
by_id.get("game_stick_base").fire("pointerdown", { pointerId: 6, stopPropagation() { }, clientX: 500, clientY: 200 });
by_id.get("game_stick_base").fire("pointermove", { pointerId: 6, clientX: 600, clientY: 300 });
by_id.get("game_stick_base").fire("pointerup", { pointerId: 6 });
check("stick moved+saved", gm.stick_x > 10 && storage.has(GAME_LAYOUT_KEY));
gm.set_editing(false);

// --- persistence roundtrip
const saved = JSON.parse(storage.get(GAME_LAYOUT_KEY));
check("state saved: preset+scale+alpha", saved.preset === "cs2" && saved.scale === 1 && saved.alpha === 1);
check("state saved: edited buttons", saved.buttons.cs2.some((b) => b.id === "gb_crouch" && b.mode === "toggle"));

// --- import validation rejects garbage
const before = gm.specs.length;
prompt_impl = () => "not json";
alerts.length = 0;
gm.import_layout();
check("import: garbage rejected", gm.specs.length === before && alerts.length === 1);
prompt_impl = () => JSON.stringify({ buttons: [{ label: "X", key: "KeyX", x: 10, y: 10, size: "sm" }, { bad: 1 }] });
gm.import_layout();
// Import replaces the current layout: 1 valid entry kept, {bad:1} skipped.
check("import: valid entries kept", gm.specs.length === 1 && gm.specs[0].label === "X" && gm.specs[0].key === "KeyX");

console.log(fails.length ? "\nFAILURES: " + fails.join(", ") : "\nALL TESTS PASSED");
process.exit(fails.length ? 1 : 0);
}
main().catch((e) => { console.error(e); process.exit(1); });
