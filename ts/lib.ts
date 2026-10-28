interface Window {
    ManagedMediaSource: any;
}

enum LogLevel {
    ERROR = 0,
    WARN,
    INFO,
    DEBUG,
    TRACE,
}

let log_pre: HTMLPreElement;
let log_level: LogLevel = LogLevel.ERROR;
let no_log_messages: boolean = true;

let fps_out: HTMLOutputElement;
let frame_count = 0;
let last_fps_calc: number = performance.now();

let check_video: HTMLInputElement;

function run(level: string) {
    window.onload = () => {
        log_pre = document.getElementById("log") as HTMLPreElement;
        log_pre.textContent = "";
        log_level = LogLevel[level];
        fps_out = document.getElementById("fps") as HTMLOutputElement;
        check_video = document.getElementById("enable_video") as HTMLInputElement;
        window.addEventListener("error", (e: ErrorEvent | Event | UIEvent) => {
            if ((e as ErrorEvent).error) {
                let err = e as ErrorEvent;
                log(LogLevel.ERROR, err.filename + ":L" + err.lineno + ":" + err.colno + ": " + err.message + " Error object: " + JSON.stringify(err.error));
            } else if ((e as Event | UIEvent).target) {
                let ev = e as Event;
                let src = (e.target as any).src;
                if (ev.target instanceof HTMLVideoElement)
                    log(LogLevel.ERROR, "Failed to decode video, try reducing resolution or disabling hardware acceleration and reload the page. Error src: " + src);
                else
                    log(LogLevel.ERROR, "Failed to obtain resource, target: " + ev.target + " type: " + ev.type + " src: " + src + " Error object: " + JSON.stringify(ev));
            } else {
                log(LogLevel.WARN, "Got unknown event: " + JSON.stringify(e));
            }
            return false;
        }, true)
        init();
    };
}

function log(level: LogLevel, msg: string) {
    if (level > log_level)
        return;

    if (level == LogLevel.TRACE)
        console.trace(msg);
    else if (level == LogLevel.DEBUG)
        console.debug(msg);
    else if (level == LogLevel.INFO)
        console.info(msg);
    else if (level == LogLevel.WARN)
        console.warn(msg);
    else if (level == LogLevel.ERROR)
        console.error(msg);

    if (no_log_messages) {
        no_log_messages = false;
        document.getElementById("log_section").classList.remove("hide");
    }
    log_pre.textContent += LogLevel[level] + ": " + msg + "\n";
}

function frame_rate_scale(x: number) {
    return Math.pow(x / 100, 1.5);
}

function frame_rate_scale_inv(x: number) {
    return 100 * Math.pow(x, 2 / 3);
}


function calc_max_video_resolution(scale: number) {
    return [
        Math.round(scale * window.innerWidth * window.devicePixelRatio),
        Math.round(scale * window.innerHeight * window.devicePixelRatio)
    ];
}

function fresh_canvas() {
    let canvas_old = document.getElementById("canvas");
    let canvas = document.createElement("canvas");
    canvas.id = canvas_old.id;
    canvas_old.classList.forEach((cls) => canvas.classList.add(cls));
    canvas_old.replaceWith(canvas);
    return canvas;
}

class Rect {
    x: number;
    y: number;
    w: number;
    h: number;
}

class CustomInputAreas {
    mouse: Rect;
    touch: Rect;
    pen: Rect;
}

class Settings {
    webSocket: WebSocket;
    checks: Map<string, HTMLInputElement>;
    capturable_select: HTMLSelectElement;
    frame_rate_input: HTMLInputElement;
    frame_rate_output: HTMLOutputElement;
    scale_video_input: HTMLInputElement;
    scale_video_output: HTMLOutputElement;
    range_min_pressure: HTMLInputElement;
    game_sensitivity_input: HTMLInputElement;
    game_sensitivity_output: HTMLOutputElement;
    check_aggressive_seek: HTMLInputElement;
    client_name_input: HTMLInputElement;
    visible: boolean;
    custom_input_areas: CustomInputAreas;
    settings: HTMLElement;

    constructor(webSocket: WebSocket) {
        this.webSocket = webSocket;
        this.checks = new Map<string, HTMLInputElement>();
        this.capturable_select = document.getElementById("window") as HTMLSelectElement;
        this.frame_rate_input = document.getElementById("frame_rate") as HTMLInputElement;
        this.frame_rate_input.min = frame_rate_scale_inv(0).toString();
        this.frame_rate_input.max = frame_rate_scale_inv(120).toString();
        this.frame_rate_output = this.frame_rate_input.nextElementSibling as HTMLOutputElement;
        this.scale_video_input = document.getElementById("scale_video") as HTMLInputElement;
        this.scale_video_output = this.scale_video_input.nextElementSibling as HTMLOutputElement;
        this.range_min_pressure = document.getElementById("min_pressure") as HTMLInputElement;
        this.game_sensitivity_input = document.getElementById("game_sensitivity") as HTMLInputElement;
        this.game_sensitivity_output = this.game_sensitivity_input.nextElementSibling as HTMLOutputElement;
        this.game_sensitivity_output.value = Number(this.game_sensitivity_input.value).toFixed(2);
        this.game_sensitivity_input.oninput = () => {
            this.game_sensitivity_output.value = Number(this.game_sensitivity_input.value).toFixed(2);
        }
        this.game_sensitivity_input.onchange = () => this.save_settings();
        this.client_name_input = document.getElementById("client_name") as HTMLInputElement;
        this.frame_rate_input.oninput = () => {
            this.frame_rate_output.value = Math.round(frame_rate_scale(this.frame_rate_input.valueAsNumber)).toString();
        }
        this.scale_video_input.oninput = () => {
            let [w, h] = calc_max_video_resolution(this.scale_video_input.valueAsNumber)
            this.scale_video_output.value = w + "x" + h
        }
        this.visible = true;

        // Settings UI
        this.settings = document.getElementById("settings");
        this.settings.onclick = (e) => e.stopPropagation();
        // On phones the sheet starts collapsed so the stream is visible.
        // The settings button in the quickbar opens it again.
        const phoneSheetLayout = window.matchMedia(
            "(max-width: 760px), (max-height: 520px) and (max-width: 900px)"
        ).matches;
        if (phoneSheetLayout) {
            this.settings.classList.add("hide");
            this.visible = false;
        }
        let handle = document.getElementById("handle");

        // Settings elements
        this.settings.querySelectorAll("input[type=checkbox]").forEach(
            (elem, _key, _parent) => this.checks.set(elem.id, elem as HTMLInputElement)
        );

        this.load_settings();

        // event handling

        // client only
        handle.onclick = () => { this.toggle() };
        this.checks.get("lefty").onchange = (e) => {
            if ((e.target as HTMLInputElement).checked)
                this.settings.classList.add("lefty");
            else
                this.settings.classList.remove("lefty");
            this.save_settings();
        }

        document.getElementById("vanish").onclick = () => {
            this.settings.classList.add("vanish");
        }

        this.checks.get("stretch").onchange = () => {
            stretch_video();
            this.save_settings();
        };

        this.checks.get("enable_debug_overlay").onchange = (e) => {
            let enabled = (e.target as HTMLInputElement).checked;
            if (enabled) {
                debug_overlay.classList.remove("hide");
            } else {
                debug_overlay.classList.add("hide");
            }
            this.save_settings();
        };

        this.check_aggressive_seek = this.checks.get("aggressive_seeking");
        this.check_aggressive_seek.onchange = () => {
            this.save_settings();
        };

        this.checks.get("enable_video").onchange = (e) => {
            let enabled = (e.target as HTMLInputElement).checked;
            document.getElementById("video").classList.toggle("vanish", !enabled);
            document.getElementById("canvas").classList.toggle("vanish", enabled);
            this.save_settings();
            if (enabled) {
                this.webSocket.send('"ResumeVideo"');
            } else {
                this.webSocket.send('"PauseVideo"');
            }
        }

        let upd_pointer = () => {
            this.save_settings();
            new PointerHandler(this.webSocket);
        }
        this.checks.get("enable_mouse").onchange = upd_pointer;
        this.checks.get("enable_stylus").onchange = upd_pointer;
        this.checks.get("enable_touch").onchange = upd_pointer;

        this.checks.get("energysaving").onchange = (e) => {
            this.save_settings();
            this.toggle_energysaving((e.target as HTMLInputElement).checked);
        };

        this.checks.get("enable_custom_input_areas").onchange = () => {
            this.save_settings();
        };

        this.frame_rate_input.onchange = () => this.save_settings();
        this.range_min_pressure.onchange = () => this.save_settings();

        // server
        let upd_server_config = () => { this.save_settings(); this.send_server_config() };
        this.checks.get("uinput_support").onchange = upd_server_config;
        this.checks.get("capture_cursor").onchange = upd_server_config;
        this.scale_video_input.onchange = upd_server_config;
        this.client_name_input.onchange = upd_server_config;
        this.frame_rate_input.onchange = upd_server_config;

        document.getElementById("refresh").onclick = () => this.webSocket.send('"GetCapturableList"');
        document.getElementById("custom_input_areas").onclick = () => {
            this.webSocket.send('"ChooseCustomInputAreas"');
        };
        this.capturable_select.onchange = () => this.send_server_config();
    }

    send_server_config() {
        let config = new Object(null);
        config["capturable_id"] = Number(this.capturable_select.value);
        for (const key of [
            "uinput_support",
            "capture_cursor"])
            config[key] = this.checks.get(key).checked;
        let [w, h] = calc_max_video_resolution(this.scale_video_input.valueAsNumber);
        config["max_width"] = w;
        config["max_height"] = h;
        config["frame_rate"] = frame_rate_scale(this.frame_rate_input.valueAsNumber);
        if (this.client_name_input.value)
            config["client_name"] = this.client_name_input.value;
        this.webSocket.send(JSON.stringify({ "Config": config }));
    }

    save_settings() {
        let settings = Object(null);
        for (const [key, elem] of this.checks.entries())
            settings[key] = elem.checked;
        settings["frame_rate"] = frame_rate_scale(this.frame_rate_input.valueAsNumber).toString();
        settings["scale_video"] = this.scale_video_input.value;
        settings["min_pressure"] = this.range_min_pressure.value;
        settings["game_sensitivity"] = this.game_sensitivity_input.value;
        settings["custom_input_areas"] = this.custom_input_areas;
        settings["client_name"] = this.client_name_input.value;
        localStorage.setItem("settings", JSON.stringify(settings));
    }

    load_settings() {
        let settings_string = localStorage.getItem("settings");
        if (settings_string === null) {
            this.frame_rate_input.value = frame_rate_scale_inv(120).toString();
            this.frame_rate_output.value = (120).toString();
            let [w, h] = calc_max_video_resolution(this.scale_video_input.valueAsNumber)
            this.scale_video_output.value = w + "x" + h;
            return;
        }
        try {
            let settings = JSON.parse(settings_string);
            for (const [key, elem] of this.checks.entries()) {
                if (typeof settings[key] === "boolean")
                    elem.checked = settings[key];
            }
            let upd_limit = settings["frame_rate"];
            if (upd_limit)
                this.frame_rate_input.value = frame_rate_scale_inv(upd_limit).toString();
            else
                this.frame_rate_input.value = frame_rate_scale_inv(120).toString();
            this.frame_rate_output.value = Math.round(frame_rate_scale(this.frame_rate_input.valueAsNumber)).toString();

            let scale_video = settings["scale_video"];
            if (scale_video)
                this.scale_video_input.value = scale_video;
            let [w, h] = calc_max_video_resolution(this.scale_video_input.valueAsNumber);
            this.scale_video_output.value = w + "x" + h;

            let min_pressure = settings["min_pressure"];
            if (min_pressure)
                this.range_min_pressure.value = min_pressure;

            let game_sensitivity = settings["game_sensitivity"];
            if (game_sensitivity)
                this.game_sensitivity_input.value = game_sensitivity;
            this.game_sensitivity_output.value = Number(this.game_sensitivity_input.value).toFixed(2);

            this.custom_input_areas = settings["custom_input_areas"];

            if (this.checks.get("lefty").checked) {
                this.settings.classList.add("lefty");
            }

            if (!this.checks.get("enable_video").checked || this.checks.get("energysaving").checked) {
                this.checks.get("enable_video").checked = false;
                if (this.checks.get("energysaving").checked)
                    this.checks.get("enable_video").disabled = true;
                document.getElementById("video").classList.add("vanish");
                document.getElementById("canvas").classList.remove("vanish");
            }

            if (this.checks.get("energysaving").checked) {
                this.toggle_energysaving(true);
            }

            if (this.checks.get("enable_debug_overlay").checked) {
                debug_overlay.classList.remove("hide");
            }


            if (document.getElementById("custom_input_areas").classList.contains("hide")) {
                this.checks.get("enable_custom_input_areas").checked = false;
            }

            let client_name = settings["client_name"];
            if (client_name)
                this.client_name_input.value = client_name;

        } catch {
            log(LogLevel.DEBUG, "Failed to load settings.")
            return;
        }
    }

    stretched_video() {
        return this.checks.get("stretch").checked
    }

    pointer_types() {
        let ptrs = [];
        if (this.checks.get("enable_mouse").checked)
            ptrs.push("mouse");
        if (this.checks.get("enable_stylus").checked)
            ptrs.push("pen");
        if (this.checks.get("enable_touch").checked)
            ptrs.push("touch");
        return ptrs;
    }

    toggle() {
        this.settings.classList.toggle("hide");
        this.visible = !this.visible;
    }

    onCapturableList(window_names: string[]) {
        let current_selection = undefined;
        if (this.capturable_select.selectedOptions[0])
            current_selection = this.capturable_select.selectedOptions[0].textContent;
        let new_index: number;
        this.capturable_select.innerText = "";
        window_names.forEach((name, i) => {
            let option = document.createElement("option");
            option.value = String(i);
            option.innerText = name;
            this.capturable_select.appendChild(option);
            if (name === current_selection)
                new_index = i;
        });
        if (new_index !== undefined)
            this.capturable_select.value = String(new_index);
        else if (current_selection)
            // Can't find the window, so don't select anything
            this.capturable_select.value = "";
    }

    toggle_energysaving(energysaving: boolean) {
        let canvas = fresh_canvas();
        if (energysaving) {
            let ctx = canvas.getContext("2d");
            ctx.fillStyle = "#000";
            ctx.fillRect(0, 0, canvas.width, canvas.height);
        }

        if (energysaving) {
            this.checks.get("enable_video").checked = false;
            this.checks.get("enable_video").disabled = true;
            this.checks.get("enable_video").dispatchEvent(new Event("change"));
        } else
            this.checks.get("enable_video").disabled = false;
        if (settings)
            new PointerHandler(this.webSocket);
    }

    video_enabled(): boolean {
        return this.checks.get("enable_video").checked;
    }
}

let settings: Settings;
let debug_overlay: HTMLElement;
let stage_overlay: StageOverlay;
let last_pointer_data: Object;
// Set once the first ConfigOk arrived; handle_disconnect clears the game
// controls of a dying session even if the page never got hidden.
let game_mode: GameMode | null = null;

class PEvent {
    event_type: string;
    pointer_id: number;
    timestamp: number;
    is_primary: boolean;
    pointer_type: string;
    button: number;
    buttons: number;
    x: number;
    y: number;
    // movement_x: number;
    // movement_y: number;
    pressure: number;
    tilt_x: number;
    tilt_y: number;
    twist: number;
    width: number;
    height: number;

    constructor(eventType: string, event: PointerEvent, targetRect: DOMRect) {
        let diag_len = Math.sqrt(targetRect.width * targetRect.width + targetRect.height * targetRect.height)
        this.event_type = eventType.toString();
        this.pointer_id = event.pointerId;
        this.timestamp = Math.round(event.timeStamp * 1000);
        this.is_primary = event.isPrimary;
        this.pointer_type = event.pointerType;
        let btn = event.button;
        // for some reason the secondary and auxiliary buttons are ordered differently for
        // the button and buttons properties
        if (btn == 2)
            btn = 1;
        else if (btn == 1)
            btn = 2;
        this.button = (btn < 0 ? 0 : 1 << btn);
        this.buttons = event.buttons;
        let x_offset = 0;
        let y_offset = 0;
        let x_scale = 1;
        let y_scale = 1;
        if (settings.checks.get("enable_custom_input_areas").checked) {
            let custom_input_area: Rect = null;
            if (event.pointerType == "mouse") {
                custom_input_area = settings.custom_input_areas.mouse;
            } else if (event.pointerType == "touch") {
                custom_input_area = settings.custom_input_areas.touch;
            } else if (event.pointerType == "pen") {
                custom_input_area = settings.custom_input_areas.pen;
            }
            if (custom_input_area) {
                x_scale = custom_input_area.w;
                y_scale = custom_input_area.h;
                x_offset = custom_input_area.x;
                y_offset = custom_input_area.y;
            }
        }
        this.x = (event.clientX - targetRect.left) / targetRect.width * x_scale + x_offset;
        this.y = (event.clientY - targetRect.top) / targetRect.height * y_scale + y_offset;
        // this.movement_x = event.movementX ? event.movementX : 0;
        // this.movement_y = event.movementY ? event.movementY : 0;
        this.pressure = Math.max(event.pressure, settings.range_min_pressure.valueAsNumber);
        this.tilt_x = event.tiltX;
        this.tilt_y = event.tiltY;
        this.width = event.width / diag_len;
        this.height = event.height / diag_len;
        this.twist = event.twist;
    }
}

class WEvent {
    dx: number;
    dy: number;
    timestamp: number;

    constructor(event: WheelEvent) {
        /* The WheelEvent can have different scrolling modes that affect how much scrolling
         * should be done. Unfortunately there is not always a way to accurately convert the scroll
         * distance into pixels. Thus the following is a guesstimate and scales the WheelEvent's
         * deltaX/Y values accordingly.
         */
        let scale = 1;
        switch (event.deltaMode) {
            case 0x01: // DOM_DELTA_LINE
                scale = 10;
                break;
            case 0x02: // DOM_DELTA_PAGE
                scale = 1000;
                break;
            default: // DOM_DELTA_PIXEL
        }
        this.dx = Math.round(scale * event.deltaX);
        this.dy = Math.round(scale * event.deltaY);
        this.timestamp = Math.round(event.timeStamp * 1000);
    }
}

// in milliseconds
const fade_time = 5000;

const vs_source = `
  attribute vec3 aVertex;
  uniform float uTime;
  varying lowp vec4 vColor;

  void main() {
    float dt = uTime - aVertex[2];
    gl_Position = vec4(aVertex[0], aVertex[1], 1.0, 1.0);
    vColor = vec4(0.0, 170.0/255.0, 1.0, 1.0) * max(1.0 - dt/${fade_time}.0, 0.0);
  }
`;

const fs_source = `
  varying lowp vec4 vColor;

  void main() {
    gl_FragColor = vColor;
  }
`;

class Painter {
    canvas: HTMLCanvasElement;
    gl: WebGLRenderingContext;

    /* Store lines currently being drawn.
     *
     * Keys are pointerIds, values are an array of the last position (x, y), thickness and event
     * time and another array with vertices to be used by webgl. Each vertex is made of 3 floats, x
     * and y coordinates and the event time. Vertices always come in pairs of two. Two such vertices
     * describe the edges of the line to be drawn with regard to it's thickness. TRIANGLE_STRIP is
     * then used to connect them and draw an actual line with some thickness depending on the
     * pressure applied.
     */
    lines_active: Map<number, [[number, number, number, number], number[]]>

    // Array of vertices that are not actively drawn anymore and do not need updates, except
    // removing them after they faded away.
    lines_old: number[][];
    vertex_attr: GLint;
    vertex_buffer: WebGLBuffer;
    time_attr: WebGLUniformLocation;
    initialized: boolean;

    constructor(canvas: HTMLCanvasElement) {
        this.canvas = canvas;
        canvas.width = window.innerWidth * window.devicePixelRatio;
        canvas.height = window.innerHeight * window.devicePixelRatio;
        this.gl = canvas.getContext("webgl");
        if (this.gl) {
            this.lines_active = new Map();
            this.lines_old = [];
            this.setupWebGL();
        }
    }

    loadShader(type, source): WebGLShader {
        let gl = this.gl;
        const shader = gl.createShader(type);
        gl.shaderSource(shader, source);
        gl.compileShader(shader);
        if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
            log(LogLevel.WARN, "Failed to compile shaders: " + gl.getShaderInfoLog(shader));
            gl.deleteShader(shader);
            return null;
        }
        return shader;
    }

    setupWebGL() {
        let gl = this.gl;
        gl.enable(gl.BLEND);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);

        const vertex_shader = this.loadShader(gl.VERTEX_SHADER, vs_source);
        const fragment_shader = this.loadShader(gl.FRAGMENT_SHADER, fs_source);
        if (!vertex_shader || !fragment_shader)
            return;
        const shader_program = gl.createProgram();
        gl.attachShader(shader_program, vertex_shader);
        gl.attachShader(shader_program, fragment_shader);
        gl.linkProgram(shader_program);

        if (!gl.getProgramParameter(shader_program, gl.LINK_STATUS)) {
            log(LogLevel.WARN, "Unable to initialize the shader program: " + gl.getProgramInfoLog(shader_program));
            return;
        }
        this.vertex_attr = gl.getAttribLocation(shader_program, "aVertex");
        this.time_attr = gl.getUniformLocation(shader_program, "uTime");
        this.vertex_buffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, this.vertex_buffer);
        gl.vertexAttribPointer(this.vertex_attr, 3, gl.FLOAT, false, 0, 0);
        gl.enableVertexAttribArray(this.vertex_attr);
        gl.useProgram(shader_program);
        this.initialized = true;
        requestAnimationFrame(() => this.render());
    }

    render() {
        // only do work if necessary
        if (!check_video.checked && (this.lines_active.size > 0 || this.lines_old.length > 0)) {
            if (this.lines_old.length > 0) {
                if (performance.now() - this.lines_old[0][this.lines_old[0].length - 1] > fade_time)
                    this.lines_old.shift();
            }
            let gl = this.gl;
            gl.viewport(0, 0, this.canvas.width, this.canvas.height);
            gl.clear(gl.COLOR_BUFFER_BIT);
            gl.uniform1f(this.time_attr, performance.now());
            gl.bindBuffer(gl.ARRAY_BUFFER, this.vertex_buffer);
            for (let vertices of this.lines_old) {
                gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(vertices), gl.DYNAMIC_DRAW);
                gl.drawArrays(gl.TRIANGLE_STRIP, 0, vertices.length / 3)
            }
            for (let [_, vertices] of this.lines_active.values()) {
                // sometimes there are no linesegments because there has been only a single
                // PointerEvent
                if (vertices.length == 0)
                    continue;
                gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(vertices), gl.DYNAMIC_DRAW);
                gl.drawArrays(gl.TRIANGLE_STRIP, 0, vertices.length / 3)
            }
        }
        requestAnimationFrame(() => this.render());
    }

    appendEventToLine(event: PointerEvent) {
        let line = this.lines_active.get(event.pointerId);
        if (!line) {
            line = [null, []];
            this.lines_active.set(event.pointerId, line)
        }
        let max_pixels = Math.max(this.canvas.width, this.canvas.height);
        let x = event.clientX * window.devicePixelRatio / this.canvas.width * 2 - 1;
        let y = 1 - event.clientY * window.devicePixelRatio / this.canvas.height * 2;
        let delta = event.pressure + 0.4;
        let t = performance.now();
        // to draw a line segment, there has to be some previous position
        if (line[0]) {
            let [x0, y0, delta0, t0] = line[0];
            // get vector perpendicular to the linesegment to calculate quadrangel around the
            // segment with appropriate thickness
            let dx = (y - y0);
            let dy = -(x - x0);
            let dd = Math.sqrt(dx ** 2 + dy ** 2);
            if (dd == 0) {
                return;
            }
            dx = dx / dd * max_pixels / this.canvas.width * 0.004;
            dy = dy / dd * max_pixels / this.canvas.height * 0.004;

            if (line[1].length == 0)
                line[1].push(
                    x0 + delta0 * dx, y0 + delta0 * dy, t0, x0 - delta0 * dx, y0 - delta0 * dy, t0,
                );
            line[1].push(
                x + delta * dx, y + delta * dy, t, x - delta * dx, y - delta * dy, t
            )
        }
        line[0] = [x, y, delta, t];
    }

    onstart(event: PointerEvent) {
        this.appendEventToLine(event);
    }

    onmove(event: PointerEvent) {
        if (this.lines_active.has(event.pointerId)) {
            const events = typeof event.getCoalescedEvents === 'function' ? event.getCoalescedEvents() : [event];
            for (const e of events) {
                this.appendEventToLine(e);
            }
        }
    }

    onstop(event: PointerEvent) {
        let lines = this.lines_active.get(event.pointerId);
        if (lines) {
            if (lines[1].length > 0)
                this.lines_old.push(lines[1]);
            this.lines_active.delete(event.pointerId);
        }
    }
}

class PointerHandler {
    webSocket: WebSocket;
    pointerTypes: string[];

    constructor(webSocket: WebSocket) {
        let video = document.getElementById("video");
        let canvas = document.getElementById("canvas");
        this.webSocket = webSocket;
        this.pointerTypes = settings.pointer_types();

        video.onpointerdown = (e) => this.onEvent(e, "pointerdown");
        video.onpointerup = (e) => this.onEvent(e, "pointerup");
        video.onpointercancel = (e) => this.onEvent(e, "pointercancel");
        video.onpointermove = (e) => this.onEvent(e, "pointermove");
        video.onpointerout = (e) => this.onEvent(e, "pointerout");
        video.onpointerleave = (e) => this.onEvent(e, "pointerleave");
        video.onpointerenter = (e) => this.onEvent(e, "pointerenter");
        video.onpointerover = (e) => this.onEvent(e, "pointerover");

        let painter: Painter;
        if (!settings.checks.get("energysaving").checked)
            painter = new Painter(canvas as HTMLCanvasElement);

        if (painter && painter.initialized) {
            canvas.onpointerdown = (e) => { this.onEvent(e, "pointerdown"); painter.onstart(e); };
            canvas.onpointerup = (e) => { this.onEvent(e, "pointerup"); painter.onstop(e); };
            canvas.onpointercancel = (e) => { this.onEvent(e, "pointercancel"); painter.onstop(e); };
            canvas.onpointermove = (e) => { this.onEvent(e, "pointermove"); painter.onmove(e); };
            canvas.onpointerout = (e) => { this.onEvent(e, "pointerout"); painter.onstop(e); };
            canvas.onpointerleave = (e) => { this.onEvent(e, "pointerleave"); painter.onstop(e); };
            canvas.onpointerenter = (e) => { this.onEvent(e, "pointerenter"); painter.onmove(e); };
            canvas.onpointerover = (e) => { this.onEvent(e, "pointerover"); painter.onmove(e); };
        } else {
            canvas.onpointerdown = (e) => this.onEvent(e, "pointerdown");
            canvas.onpointerup = (e) => this.onEvent(e, "pointerup");
            canvas.onpointercancel = (e) => this.onEvent(e, "pointercancel");
            canvas.onpointermove = (e) => this.onEvent(e, "pointermove");
            canvas.onpointerout = (e) => this.onEvent(e, "pointerout");
            canvas.onpointerleave = (e) => this.onEvent(e, "pointerleave");
            canvas.onpointerenter = (e) => this.onEvent(e, "pointerenter");
            canvas.onpointerover = (e) => this.onEvent(e, "pointerover");
        }

        // This is a workaround for the following Safari/WebKit bug:
        // https://bugs.webkit.org/show_bug.cgi?id=217430
        // I have no idea why this works but it does.
        video.ontouchmove = (e) => e.preventDefault();
        canvas.ontouchmove = (e) => e.preventDefault();

        for (let elem of [video, canvas]) {
            elem.onwheel = (e) => {
                this.webSocket.send(JSON.stringify({ "WheelEvent": new WEvent(e) }));
            }
        }
    }

    onEvent(event: PointerEvent, event_type: string) {
        if (settings.checks.get("enable_debug_overlay").checked) {
            let props = [
                "altKey",
                "altitudeAngle",
                "azimuthAngle",
                "button",
                "buttons",
                "clientX",
                "clientY",
                "ctrlKey",
                "height",
                "isPrimary",
                "metaKey",
                "movementX",
                "movementY",
                "offsetX",
                "offsetY",
                "pageX",
                "pageY",
                "pointerId",
                "pointerType",
                "pressure",
                "screenX",
                "screenY",
                "shiftKey",
                "tangentialPressure",
                "tiltX",
                "tiltY",
                "timeStamp",
                "twist",
                "type",
                "width",
                "x",
                "y",
            ];
            if (!last_pointer_data) {
                last_pointer_data = {};
                for (let prop of props) {
                    let span_id = `prop_${prop}_span`;
                    let span = document.getElementById(span_id);
                    span = document.createElement("span");
                    span.id = span_id;
                    debug_overlay.appendChild(span);
                    debug_overlay.appendChild(document.createElement("br"));
                }
            }
            for (let prop of props) {
                let span_id = `prop_${prop}_span`;
                let span = document.getElementById(span_id);
                let v = event[prop];
                span.textContent = `${prop}: ${v}`;
                if (last_pointer_data[prop] == v) {
                    span.classList.remove("updated");
                } else {
                    span.classList.add("updated");
                    last_pointer_data[prop] = v;
                }
            }
        }
        if (this.pointerTypes.includes(event.pointerType)) {
            let rect = (event.target as HTMLElement).getBoundingClientRect();
            const events = event_type === "pointermove" && typeof event.getCoalescedEvents === 'function' ? event.getCoalescedEvents() : [event];
            for (let event of events) {
                this.webSocket.send(
                    JSON.stringify(
                        {
                            "PointerEvent": new PEvent(
                                event_type,
                                event,
                                rect
                            )
                        }
                    )
                );
            }
            if (settings.visible) {
                settings.toggle();
            }
        }
    }
}

class KEvent {
    event_type: string;
    code: string;
    key: string;
    location: number;
    alt: boolean;
    ctrl: boolean;
    shift: boolean;
    meta: boolean;

    constructor(event_type: string, event: KeyboardEvent) {
        this.event_type = event_type;
        this.code = event.code;
        this.key = event.key;
        this.location = event.location;
        this.alt = event.altKey;
        this.ctrl = event.ctrlKey;
        this.shift = event.shiftKey;
        this.meta = event.metaKey;
    }
}

class KeyboardHandler {
    webSocket: WebSocket;

    constructor(webSocket: WebSocket) {
        this.webSocket = webSocket;

        let d = document;
        let s = document.getElementById("settings")

        // Consume all KeyboardEvents, except the settings menu is open.
        // this avoids making the main/video/canvas element focusable by using
        // a tabindex which interferes with PointerEvent than can be considered
        // hovering.

        function settings_hidden() {
            return s.classList.contains("hide") || s.classList.contains("vanish");
        }

        d.onkeydown = (e) => {
            if (!settings_hidden())
                return true;
            if (e.repeat)
                return this.onEvent(e, "repeat");
            return this.onEvent(e, "down");
        };
        d.onkeyup = (e) => {
            if (!settings_hidden())
                return true;
            return this.onEvent(e, "up");
        };
        d.onkeypress = (e) => {
            if (!settings_hidden())
                return true;
            e.preventDefault();
            e.stopPropagation();
            return false;
        };
    }

    onEvent(event: KeyboardEvent, event_type: string) {
        this.webSocket.send(JSON.stringify({ "KeyboardEvent": new KEvent(event_type, event) }));
        event.preventDefault();
        event.stopPropagation();
        return false;
    }
}

class SoftKeyboard {
    webSocket: WebSocket;
    bar: HTMLElement;
    input: HTMLInputElement;
    closeButton: HTMLButtonElement;
    toggleButton: HTMLButtonElement;
    enabled: boolean;
    // timestamp of the last deactivation; used to detect a click that was
    // caused by the same tap as the blur that just deactivated the funnel
    deactivated_at: number;
    // content of the funnel field that was already forwarded to the host
    committed: string;

    constructor(webSocket: WebSocket) {
        this.webSocket = webSocket;
        this.bar = document.getElementById("softkb_bar");
        this.input = document.getElementById("softkb_input") as HTMLInputElement;
        this.toggleButton = document.getElementById("qk_softkb") as HTMLButtonElement;
        this.closeButton = document.getElementById("softkb_close") as HTMLButtonElement;
        this.enabled = false;
        this.deactivated_at = 0;
        this.committed = "";
        this.toggleButton.setAttribute("aria-pressed", "false");

        this.toggleButton.onclick = (e) => {
            e.stopPropagation();
            if (this.enabled) {
                // Closing always goes through a blur: on mobile that is what
                // actually dismisses the OS keyboard and the blur handler
                // below does the rest. If the field is not focused anymore
                // (e.g. the keyboard was dismissed with a system gesture)
                // blur will never fire, so deactivate directly.
                if (!this.dismiss_if_active()) {
                    this.deactivate();
                }
            } else if (Date.now() - this.deactivated_at < 400) {
                // The tap that produced this click blurred the field first
                // (focus moves to the button), the blur listener below
                // deactivated the funnel and only afterwards the click
                // arrived. That tap meant CLOSE, so do not immediately
                // reopen - otherwise the second press never closes anything
                // because deactivate+activate undo each other.
            } else {
                this.activate();
            }
        };

        this.closeButton.onclick = (e) => {
            e.stopPropagation();
            if (!this.dismiss_if_active()) {
                this.deactivate();
            }
        };

        // Whenever the field loses focus (close button, tapping the stream,
        // Android dismissing the keyboard with a system gesture) the funnel
        // is deactivated synchronously. Without this the next tap on the
        // stream would first blur the field mid-gesture and, on iOS, after an
        // abrupt keyboard dismissal the gestures on the stream would be
        // swallowed completely, hence the dead mouse after closing the
        // keyboard.
        this.input.addEventListener("blur", () => {
            this.deactivate();
        });

        // Tapping the stream dismisses the funnel before the raw pointer
        // sequence is interpreted. Like remote_control this issues the blur
        // at touchstart with preventDefault, so refocusing and the layout
        // resize caused by the keyboard closing can not cancel the gesture
        // and the mouse stays usable from the very first contact.
        const video = document.getElementById("video");
        if (video) {
            video.addEventListener("pointerdown", () => {
                this.dismiss_if_active();
            });
            video.addEventListener("touchstart", (e) => {
                if (this.dismiss_if_active()) {
                    e.preventDefault();
                }
            }, { passive: false });
        }

        this.input.onkeydown = (e) => {
            e.stopPropagation();
            // keyCode 229 is Chrome's composition marker, the composed text
            // is forwarded via its own input event after compositionend.
            if (e.isComposing || e.keyCode === 229) {
                return true;
            }
            if (e.key === "Enter") {
                e.preventDefault();
                this.commit();
                this.send_special_key("Enter");
                return false;
            }
            // When the field is empty the browser has nothing to delete and
            // the following input event would never fire, so backspace has to
            // be sent right here. With a non-empty field the deletion happens
            // inside the field and the input handler below forwards it via
            // the grapheme diff instead.
            if (e.key === "Backspace" && this.input.value.length === 0) {
                e.preventDefault();
                this.send_special_key("Backspace");
            }
            return true;
        };

        // Text is funneled through the input event and the field value is
        // NEVER reset while typing: touching the value of a live field desyn-
        // chronizes mobile keyboards - Chrome stops delivering events after
        // the first character and IME compositions get lost. Instead the
        // shown value is grapheme-diffed against everything already forwarded
        // and only that edit is sent, the content simply keeps accumulating.
        this.input.addEventListener("input", (e) => {
            const event = e as InputEvent;
            // While an IME composition is running (Gboard etc.) the value is
            // not final yet; the commit fires its own plain input event after
            // compositionend which is used instead.
            if (event.isComposing) {
                return;
            }
            this.commit();
        });
        this.input.addEventListener("compositionend", () => {
            this.commit();
        });
    }

    // Grapheme-aware diff of the field value against the already forwarded
    // content, ported from remote_control's commitKeyboardComposition. The
    // result is sent as a TextEdit: right `left` characters of the caret
    // keep their position, `backspace` characters are deleted, `text`
    // is inserted in place and the caret ends up `right` positions from
    // the start of the replacement - the server applies the same recipe on
    // the host. Insertions, deletions and arbitrary replacements (autocor-
    // rect, swipe input, emoji picker) all resolve to the minimal edit.
    commit() {
        const before = split_graphemes(this.committed);
        const after = split_graphemes(this.input.value);
        let prefix = 0;
        let suffix = 0;
        while (prefix < before.length && prefix < after.length
            && before[prefix] === after[prefix]) {
            prefix++;
        }
        while (suffix < before.length - prefix && suffix < after.length - prefix
            && before[before.length - 1 - suffix] === after[after.length - 1 - suffix]) {
            suffix++;
        }
        if (prefix !== before.length || prefix !== after.length) {
            this.send_text_edit(
                suffix,
                before.length - prefix - suffix,
                after.slice(prefix, after.length - suffix).join(""),
                suffix
            );
        }
        this.committed = this.input.value;
    }

    // Blur only when the funnel field is actually focused. Returns whether
    // an active funnel was just dismissed.
    dismiss_if_active() {
        if (document.activeElement !== this.input) {
            return false;
        }
        this.input.blur();
        return true;
    }

    activate() {
        this.enabled = true;
        this.bar.classList.remove("hide");
        this.toggleButton.classList.add("active");
        this.toggleButton.setAttribute("aria-pressed", "true");
        // The funnel deliberately keeps its content across sessions: never
        // reset the value here. On a live field that desynchronizes mobile
        // keyboards - after reopening, the IME replays everything it still
        // held, which then types the previous prompt into the host. Keeping
        // value and committed identical makes the reopen seamless and the
        // grapheme diff a no-op. Place the caret at the end so continuing
        // typing appends, like on the phone's own keyboard.
        this.input.focus();
        const len = this.input.value.length;
        this.input.setSelectionRange(len, len);
    }

    deactivate() {
        this.enabled = false;
        this.deactivated_at = Date.now();
        this.bar.classList.add("hide");
        this.toggleButton.classList.remove("active");
        this.toggleButton.setAttribute("aria-pressed", "false");
        // Deliberately NOT resetting the field: value and committed stay in
        // sync so the next session continues exactly where the user left
        // instead of replaying stale text into the host.
    }

    send_text_edit(left: number, backspace: number, text: string, right: number) {
        this.webSocket.send(JSON.stringify(
            { "TextEdit": { "left": left, "backspace": backspace, "text": text, "right": right } }
        ));
    }

    send_special_key(code: string) {
        // location 0 = standard, modifiers are never active here
        const key = {
            event_type: "down", code: code, key: code, location: 0,
            alt: false, ctrl: false, shift: false, meta: false,
        };
        this.webSocket.send(JSON.stringify({ "KeyboardEvent": key }));
        key.event_type = "up";
        this.webSocket.send(JSON.stringify({ "KeyboardEvent": key }));
    }
}


// Grapheme clustering for the funnel diff. Splitting by code points alone
// would tear apart surrogate pairs and ZWJ sequences that compose to a single
// visible character (flags, skin toned emoji).
function split_graphemes(text: string): string[] {
    // Intl.Segmenter is not part of the es2020 TS lib, hence the casts.
    const segmenter = typeof (Intl as any).Segmenter === "function"
        ? new (Intl as any).Segmenter(undefined, { granularity: "grapheme" }) : null;
    if (segmenter) {
        return [...segmenter.segment(text)].map((part: any) => part.segment);
    }
    return Array.from(text);
}

// Shared PC-audio stream toggle. Game mode hides the quickbar, so it gets
// its own speaker button wired to the same stream: there is exactly one
// <audio> element no matter which button was used, and every wired button
// mirrors the on/off state.
let audio_element: HTMLAudioElement | null = null;
const audio_toggle_buttons: HTMLButtonElement[] = [];

function reflect_audio_buttons() {
    const on = audio_element !== null;
    for (const btn of audio_toggle_buttons) {
        btn.classList.toggle("active", on);
        btn.setAttribute("aria-pressed", on ? "true" : "false");
    }
}

function stop_audio_stream() {
    if (audio_element) {
        audio_element.pause();
        audio_element.removeAttribute("src");
        audio_element.load();
        audio_element.remove();
        audio_element = null;
    }
    reflect_audio_buttons();
}

function wire_audio_button(btn: HTMLButtonElement) {
    btn.oncontextmenu = (e) => { e.preventDefault(); return false; };
    btn.onclick = (e) => {
        e.stopPropagation();
        if (audio_element) {
            stop_audio_stream();
            return;
        }
        // Creating the element inside the click handler doubles as the
        // user gesture required by autoplay policies; removing it again
        // aborts the connection, which makes the server stop capturing
        // and encoding. The query string carries the access code over.
        audio_element = document.createElement("audio");
        audio_element.src = "/audio_stream" + window.location.search;
        audio_element.style.display = "none";
        document.body.appendChild(audio_element);
        reflect_audio_buttons();
        audio_element.play().catch((err) => {
            log(LogLevel.WARN, "Audio streaming failed to start: " + err);
            stop_audio_stream();
        });
    };
    audio_toggle_buttons.push(btn);
    reflect_audio_buttons();
}

// Floating quick action buttons rendered over the video: mouse button
// hold-to-press buttons act at the current cursor position of the host via
// ClickEvent, key buttons send full press+release KeyboardEvents and the
// keyboard button toggles the soft keyboard bar.
class QuickBar {
    webSocket: WebSocket;

    constructor(webSocket: WebSocket) {
        this.webSocket = webSocket;


        // The F11 button toggles fullscreen of this page on the device
        // itself, independent from the computer.
        const btn = document.getElementById("qk_f11") as HTMLButtonElement;
        btn.oncontextmenu = (e) => { e.preventDefault(); return false; };
        btn.onclick = (e) => {
            e.stopPropagation();
            if (document.fullscreenElement) {
                document.exitFullscreen();
            } else {
                document.documentElement.requestFullscreen();
            }
        };
        document.addEventListener("fullscreenchange", () => {
            const active = document.fullscreenElement !== null;
            btn.classList.toggle("active", active);
            btn.setAttribute("aria-pressed", active ? "true" : "false");
        });

        wire_audio_button(document.getElementById("qk_audio") as HTMLButtonElement);
    }
}

// Game mode: a Standoff-style control layer over the video. The overlay
// background is the look surface - dragging it sends relative MouseMoveEvents
// so the host view turns like with a real mouse. The joystick translates its
// stick vector into held WASD KeyboardEvents and the action buttons inject
// mouse buttons (ClickEvent) or keys (KeyboardEvent); every press is paired
// with a release so buttons can be held like real keys.
interface GameButtonSpec {
    id: string;
    label: string;
    // Exactly one of these is set: the mouse button or the KeyboardEvent
    // code the button holds down while it is touched.
    mouse?: "left" | "middle" | "right";
    key?: string;
    // Button center in percent of the viewport.
    x: number;
    y: number;
    size: "sm" | "md" | "lg";
}

// Preset layouts for popular games. Every preset stays fully editable:
// the edit mode can move, resize, rebind, add and delete buttons and the
// result is kept per preset in localStorage.
interface GamePreset {
    name: string;
    buttons: GameButtonSpec[];
}

const GAME_PRESETS: Record<string, GamePreset> = {
    // Default CS2 layout: the right thumb aims and fires, the left thumb
    // walks, utilities sit along the edges and weapon slots run along the
    // bottom.
    cs2: {
        name: "CS2 / CS:GO", buttons: [
            // right cluster
            { id: "gb_fire", label: "FIRE", mouse: "left", x: 84, y: 70, size: "lg" },
            { id: "gb_aim", label: "AIM", mouse: "right", x: 70, y: 57, size: "md" },
            { id: "gb_jump", label: "JUMP", key: "Space", x: 88, y: 50, size: "md" },
            { id: "gb_crouch", label: "CRCH", key: "ControlLeft", x: 70, y: 84, size: "md" },
            // left cluster
            { id: "gb_walk", label: "WALK", key: "ShiftLeft", x: 13, y: 48, size: "sm" },
            { id: "gb_reload", label: "R", key: "KeyR", x: 26, y: 40, size: "sm" },
            { id: "gb_use", label: "E", key: "KeyE", x: 10, y: 28, size: "sm" },
            { id: "gb_drop", label: "G", key: "KeyG", x: 21, y: 20, size: "sm" },
            // weapon slots along the bottom center
            { id: "gb_slot1", label: "1", key: "Digit1", x: 44, y: 90, size: "sm" },
            { id: "gb_slot2", label: "2", key: "Digit2", x: 50, y: 90, size: "sm" },
            { id: "gb_slot3", label: "3", key: "Digit3", x: 56, y: 90, size: "sm" },
            { id: "gb_slot4", label: "4", key: "Digit4", x: 62, y: 90, size: "sm" },
            { id: "gb_lastw", label: "Q", key: "KeyQ", x: 68, y: 90, size: "sm" },
            // top row: scoreboard, buy menu and the game menu
            { id: "gb_score", label: "TAB", key: "Tab", x: 7, y: 8, size: "sm" },
            { id: "gb_buy", label: "B", key: "KeyB", x: 14, y: 8, size: "sm" },
            { id: "gb_esc", label: "ESC", key: "Escape", x: 21, y: 8, size: "sm" },
        ]
    },
    valorant: {
        name: "Valorant", buttons: [
            { id: "gb_fire", label: "FIRE", mouse: "left", x: 84, y: 70, size: "lg" },
            { id: "gb_aim", label: "AIM", mouse: "right", x: 70, y: 57, size: "md" },
            { id: "gb_jump", label: "JUMP", key: "Space", x: 88, y: 50, size: "md" },
            { id: "gb_crouch", label: "CRCH", key: "ControlLeft", x: 70, y: 84, size: "md" },
            { id: "gb_walk", label: "WALK", key: "ShiftLeft", x: 13, y: 48, size: "sm" },
            { id: "gb_reload", label: "R", key: "KeyR", x: 26, y: 34, size: "sm" },
            { id: "gb_use", label: "F", key: "KeyF", x: 10, y: 26, size: "sm" },
            { id: "gb_e", label: "E", key: "KeyE", x: 17, y: 26, size: "sm" },
            { id: "gb_q", label: "Q", key: "KeyQ", x: 10, y: 16, size: "sm" },
            { id: "gb_cab", label: "C", key: "KeyC", x: 17, y: 16, size: "sm" },
            { id: "gb_ult", label: "X", key: "KeyX", x: 24, y: 16, size: "sm" },
            { id: "gb_slot1", label: "1", key: "Digit1", x: 44, y: 90, size: "sm" },
            { id: "gb_slot2", label: "2", key: "Digit2", x: 50, y: 90, size: "sm" },
            { id: "gb_slot3", label: "3", key: "Digit3", x: 56, y: 90, size: "sm" },
            { id: "gb_slot4", label: "4", key: "Digit4", x: 62, y: 90, size: "sm" },
            { id: "gb_score", label: "TAB", key: "Tab", x: 7, y: 8, size: "sm" },
            { id: "gb_buy", label: "B", key: "KeyB", x: 14, y: 8, size: "sm" },
            { id: "gb_esc", label: "ESC", key: "Escape", x: 21, y: 8, size: "sm" },
        ]
    },
    fortnite: {
        name: "Fortnite", buttons: [
            { id: "gb_fire", label: "FIRE", mouse: "left", x: 84, y: 70, size: "lg" },
            { id: "gb_aim", label: "AIM", mouse: "right", x: 70, y: 57, size: "md" },
            { id: "gb_jump", label: "JUMP", key: "Space", x: 88, y: 50, size: "md" },
            { id: "gb_crouch", label: "CRCH", key: "ControlLeft", x: 70, y: 84, size: "md" },
            { id: "gb_run", label: "RUN", key: "ShiftLeft", x: 13, y: 48, size: "sm" },
            { id: "gb_reload", label: "R", key: "KeyR", x: 26, y: 40, size: "sm" },
            { id: "gb_editk", label: "ED", key: "KeyG", x: 10, y: 28, size: "sm" },
            { id: "gb_map", label: "MAP", key: "KeyM", x: 21, y: 20, size: "sm" },
            // building keys above the weapon slots
            { id: "gb_wall", label: "WALL", key: "F1", x: 58, y: 34, size: "sm" },
            { id: "gb_floor", label: "FLR", key: "F2", x: 66, y: 28, size: "sm" },
            { id: "gb_stair", label: "STA", key: "F3", x: 74, y: 34, size: "sm" },
            { id: "gb_roof", label: "ROOF", key: "F4", x: 66, y: 42, size: "sm" },
            { id: "gb_slot1", label: "1", key: "Digit1", x: 44, y: 90, size: "sm" },
            { id: "gb_slot2", label: "2", key: "Digit2", x: 50, y: 90, size: "sm" },
            { id: "gb_slot3", label: "3", key: "Digit3", x: 56, y: 90, size: "sm" },
            { id: "gb_slot4", label: "4", key: "Digit4", x: 62, y: 90, size: "sm" },
            { id: "gb_slot5", label: "5", key: "Digit5", x: 68, y: 90, size: "sm" },
            { id: "gb_inv", label: "INV", key: "KeyI", x: 7, y: 8, size: "sm" },
            { id: "gb_esc", label: "ESC", key: "Escape", x: 14, y: 8, size: "sm" },
        ]
    },
    warzone: {
        name: "CoD: Warzone", buttons: [
            { id: "gb_fire", label: "FIRE", mouse: "left", x: 84, y: 70, size: "lg" },
            { id: "gb_aim", label: "AIM", mouse: "right", x: 70, y: 57, size: "md" },
            { id: "gb_jump", label: "JUMP", key: "Space", x: 88, y: 50, size: "md" },
            { id: "gb_crouch", label: "CRCH", key: "KeyC", x: 70, y: 84, size: "md" },
            { id: "gb_prone", label: "PRNE", key: "ControlLeft", x: 78, y: 88, size: "sm" },
            { id: "gb_run", label: "RUN", key: "ShiftLeft", x: 13, y: 48, size: "sm" },
            { id: "gb_reload", label: "R", key: "KeyR", x: 26, y: 40, size: "sm" },
            { id: "gb_use", label: "F", key: "KeyF", x: 10, y: 28, size: "sm" },
            { id: "gb_tac", label: "TAC", key: "KeyG", x: 21, y: 20, size: "sm" },
            { id: "gb_leth", label: "LETH", key: "KeyQ", x: 10, y: 18, size: "sm" },
            { id: "gb_slot1", label: "1", key: "Digit1", x: 44, y: 90, size: "sm" },
            { id: "gb_slot2", label: "2", key: "Digit2", x: 50, y: 90, size: "sm" },
            { id: "gb_slot3", label: "3", key: "Digit3", x: 56, y: 90, size: "sm" },
            { id: "gb_slot4", label: "4", key: "Digit4", x: 62, y: 90, size: "sm" },
            { id: "gb_score", label: "TAB", key: "Tab", x: 7, y: 8, size: "sm" },
            { id: "gb_esc", label: "ESC", key: "Escape", x: 14, y: 8, size: "sm" },
        ]
    },
    apex: {
        name: "Apex Legends", buttons: [
            { id: "gb_fire", label: "FIRE", mouse: "left", x: 84, y: 70, size: "lg" },
            { id: "gb_aim", label: "AIM", mouse: "right", x: 70, y: 57, size: "md" },
            { id: "gb_jump", label: "JUMP", key: "Space", x: 88, y: 50, size: "md" },
            { id: "gb_crouch", label: "CRCH", key: "KeyC", x: 70, y: 84, size: "md" },
            { id: "gb_run", label: "RUN", key: "ShiftLeft", x: 13, y: 48, size: "sm" },
            { id: "gb_reload", label: "R", key: "KeyR", x: 26, y: 40, size: "sm" },
            { id: "gb_use", label: "E", key: "KeyE", x: 10, y: 28, size: "sm" },
            { id: "gb_tac", label: "TAC", key: "KeyQ", x: 21, y: 20, size: "sm" },
            { id: "gb_ult", label: "ULT", key: "KeyZ", x: 10, y: 16, size: "sm" },
            { id: "gb_gren", label: "GRN", key: "KeyG", x: 28, y: 16, size: "sm" },
            { id: "gb_slot1", label: "1", key: "Digit1", x: 44, y: 90, size: "sm" },
            { id: "gb_slot2", label: "2", key: "Digit2", x: 50, y: 90, size: "sm" },
            { id: "gb_melee", label: "V", key: "KeyV", x: 56, y: 90, size: "sm" },
            { id: "gb_heal", label: "4", key: "Digit4", x: 62, y: 90, size: "sm" },
            { id: "gb_score", label: "TAB", key: "Tab", x: 7, y: 8, size: "sm" },
            { id: "gb_map", label: "MAP", key: "KeyM", x: 14, y: 8, size: "sm" },
            { id: "gb_esc", label: "ESC", key: "Escape", x: 21, y: 8, size: "sm" },
        ]
    },
    pubg: {
        name: "PUBG", buttons: [
            { id: "gb_fire", label: "FIRE", mouse: "left", x: 84, y: 70, size: "lg" },
            { id: "gb_aim", label: "AIM", mouse: "right", x: 70, y: 57, size: "md" },
            { id: "gb_jump", label: "JUMP", key: "Space", x: 88, y: 50, size: "md" },
            { id: "gb_crouch", label: "CRCH", key: "KeyC", x: 70, y: 84, size: "md" },
            { id: "gb_prone", label: "PRNE", key: "KeyZ", x: 78, y: 88, size: "sm" },
            { id: "gb_walk", label: "WALK", key: "ShiftLeft", x: 13, y: 48, size: "sm" },
            { id: "gb_reload", label: "R", key: "KeyR", x: 26, y: 40, size: "sm" },
            { id: "gb_use", label: "F", key: "KeyF", x: 10, y: 28, size: "sm" },
            { id: "gb_gren", label: "GRN", key: "KeyG", x: 21, y: 20, size: "sm" },
            { id: "gb_slot1", label: "1", key: "Digit1", x: 44, y: 90, size: "sm" },
            { id: "gb_slot2", label: "2", key: "Digit2", x: 50, y: 90, size: "sm" },
            { id: "gb_slot3", label: "3", key: "Digit3", x: 56, y: 90, size: "sm" },
            { id: "gb_slot4", label: "4", key: "Digit4", x: 62, y: 90, size: "sm" },
            { id: "gb_slot5", label: "5", key: "Digit5", x: 68, y: 90, size: "sm" },
            { id: "gb_map", label: "MAP", key: "KeyM", x: 7, y: 8, size: "sm" },
            { id: "gb_inv", label: "INV", key: "KeyI", x: 14, y: 8, size: "sm" },
            { id: "gb_esc", label: "ESC", key: "Escape", x: 21, y: 8, size: "sm" },
        ]
    },
    minecraft: {
        name: "Minecraft", buttons: [
            { id: "gb_atk", label: "ATK", mouse: "left", x: 84, y: 70, size: "lg" },
            { id: "gb_use", label: "USE", mouse: "right", x: 70, y: 57, size: "md" },
            { id: "gb_jump", label: "JUMP", key: "Space", x: 88, y: 50, size: "md" },
            { id: "gb_sneak", label: "SNK", key: "ShiftLeft", x: 70, y: 84, size: "md" },
            { id: "gb_sprint", label: "SPR", key: "ControlLeft", x: 13, y: 48, size: "sm" },
            { id: "gb_drop", label: "Q", key: "KeyQ", x: 26, y: 40, size: "sm" },
            { id: "gb_inv", label: "INV", key: "KeyE", x: 10, y: 28, size: "sm" },
            { id: "gb_chat", label: "CHAT", key: "KeyT", x: 21, y: 20, size: "sm" },
            { id: "gb_off", label: "OFF", key: "KeyF", x: 10, y: 18, size: "sm" },
            { id: "gb_slot1", label: "1", key: "Digit1", x: 44, y: 90, size: "sm" },
            { id: "gb_slot2", label: "2", key: "Digit2", x: 50, y: 90, size: "sm" },
            { id: "gb_slot3", label: "3", key: "Digit3", x: 56, y: 90, size: "sm" },
            { id: "gb_slot4", label: "4", key: "Digit4", x: 62, y: 90, size: "sm" },
            { id: "gb_slot5", label: "5", key: "Digit5", x: 68, y: 90, size: "sm" },
            { id: "gb_slot6", label: "6", key: "Digit6", x: 74, y: 90, size: "sm" },
            { id: "gb_esc", label: "ESC", key: "Escape", x: 7, y: 8, size: "sm" },
        ]
    },
    lol: {
        name: "League of Legends", buttons: [
            { id: "gb_q", label: "Q", key: "KeyQ", x: 84, y: 70, size: "lg" },
            { id: "gb_w", label: "W", key: "KeyW", x: 70, y: 57, size: "md" },
            { id: "gb_e", label: "E", key: "KeyE", x: 88, y: 50, size: "md" },
            { id: "gb_r", label: "R", key: "KeyR", x: 70, y: 84, size: "md" },
            { id: "gb_d", label: "D", key: "KeyD", x: 88, y: 84, size: "sm" },
            { id: "gb_f", label: "F", key: "KeyF", x: 78, y: 88, size: "sm" },
            { id: "gb_b", label: "B", key: "KeyB", x: 60, y: 74, size: "sm" },
            { id: "gb_stop", label: "S", key: "KeyS", x: 26, y: 40, size: "sm" },
            { id: "gb_item1", label: "1", key: "Digit1", x: 38, y: 90, size: "sm" },
            { id: "gb_item2", label: "2", key: "Digit2", x: 44, y: 90, size: "sm" },
            { id: "gb_item3", label: "3", key: "Digit3", x: 50, y: 90, size: "sm" },
            { id: "gb_item4", label: "4", key: "Digit4", x: 56, y: 90, size: "sm" },
            { id: "gb_item5", label: "5", key: "Digit5", x: 62, y: 90, size: "sm" },
            { id: "gb_item6", label: "6", key: "Digit6", x: 68, y: 90, size: "sm" },
            { id: "gb_score", label: "TAB", key: "Tab", x: 7, y: 8, size: "sm" },
            { id: "gb_lock", label: "LOCK", key: "KeyY", x: 14, y: 8, size: "sm" },
            { id: "gb_esc", label: "ESC", key: "Escape", x: 21, y: 8, size: "sm" },
        ]
    },
};

const GAME_DEFAULT_PRESET = "cs2";

// Persisted game layout state: the chosen preset, the global button size
// scale, the joystick position and per-preset edited button lists.
interface GameLayoutState {
    preset: string;
    scale: number;
    stick: { x: number, y: number };
    buttons: Record<string, GameButtonSpec[]>;
}

const GAME_LAYOUT_KEY = "game_layout";

// Keys offered in the button editor; KeyboardEvent codes, not characters,
// because the host maps them to virtual keys independent of its layout.
const GAME_EDIT_KEYS = (() => {
    const codes: string[] = [];
    for (let i = 65; i <= 90; i++)
        codes.push("Key" + String.fromCharCode(i));
    for (let i = 0; i <= 9; i++)
        codes.push("Digit" + i);
    codes.push("Space", "Tab", "Escape", "Enter", "CapsLock", "Backspace",
        "ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft",
        "Backquote", "Minus", "Equal", "BracketLeft", "BracketRight",
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8");
    return codes;
})();

function game_key_label(code: string): string {
    const simple = code.match(/^(?:Key|Digit)(.)$/);
    if (simple)
        return simple[1];
    const named: Record<string, string> = {
        Space: "SPACE", Tab: "TAB", Escape: "ESC", Enter: "ENTER",
        CapsLock: "CAPS", Backspace: "BKSP", Backquote: "`", Minus: "-",
        Equal: "=", BracketLeft: "[", BracketRight: "]",
        ShiftLeft: "SHIFT", ShiftRight: "RSHFT",
        ControlLeft: "CTRL", ControlRight: "RCTRL", AltLeft: "ALT",
    };
    return named[code] ?? code;
}

function clone_buttons(specs: GameButtonSpec[]): GameButtonSpec[] {
    return specs.map((s) => ({ ...s }));
}

// The action select encodes the binding as "mouse:<button>" / "key:<code>".
function game_button_action(spec: GameButtonSpec): string {
    return spec.mouse ? "mouse:" + spec.mouse : "key:" + (spec.key ?? "Space");
}

class GameMode {
    webSocket: WebSocket;
    overlay: HTMLElement;
    stick_base: HTMLElement;
    stick_knob: HTMLElement;
    enabled: boolean = false;

    // Active preset id and its working button list. Edits land in `specs`
    // and are copied into `layouts` on every save, so each preset keeps its
    // own customized arrangement across reloads.
    preset: string = GAME_DEFAULT_PRESET;
    specs: GameButtonSpec[] = [];
    layouts: Record<string, GameButtonSpec[]> = {};
    // Global button size multiplier from the settings slider.
    scale: number = 1;
    // Joystick position in percent; null falls back to the CSS default.
    stick_x: number | null = null;
    stick_y: number | null = null;

    // Layout editing state.
    editing: boolean = false;
    selected_id: string | null = null;
    drag: { id: string, off_x: number, off_y: number } | null = null;
    edit_bar: HTMLElement;
    sel_panel: HTMLElement;
    label_input: HTMLInputElement;
    action_select: HTMLSelectElement;
    size_select: HTMLSelectElement;

    // one pointer drags the look surface at a time
    look_pointer: number | null = null;
    look_last_x: number = 0;
    look_last_y: number = 0;
    // fractional pixels not sent yet, so the many small deltas of a fast
    // swipe are not lost to rounding
    look_rest_x: number = 0;
    look_rest_y: number = 0;

    stick_pointer: number | null = null;
    stick_center_x: number = 0;
    stick_center_y: number = 0;
    // WASD currently held by the joystick
    stick_keys: Set<string> = new Set();
    // Refcounted held input: the joystick's slow-walk and the WALK button
    // can hold the same key (ShiftLeft), so a key is released only when its
    // last holder lets go.
    key_refs: Map<string, number> = new Map();
    mouse_refs: Map<string, number> = new Map();
    wake_lock: any = null;

    constructor(webSocket: WebSocket) {
        this.webSocket = webSocket;
        this.overlay = document.getElementById("game_overlay") as HTMLElement;
        this.stick_base = document.getElementById("game_stick_base") as HTMLElement;
        this.stick_knob = document.getElementById("game_stick") as HTMLElement;

        this.load_state();
        this.apply_scale();
        this.apply_stick_position();
        this.render_buttons();
        this.build_edit_ui();

        // Preset, button size and layout editing live in the settings sheet.
        const preset_select = document.getElementById("game_preset") as HTMLSelectElement;
        preset_select.value = this.preset;
        preset_select.onchange = () => this.load_preset(preset_select.value);

        const size_input = document.getElementById("game_btn_size") as HTMLInputElement;
        const size_output = size_input.nextElementSibling as HTMLOutputElement;
        size_input.value = String(this.scale);
        size_output.value = Math.round(this.scale * 100) + "%";
        size_input.oninput = () => {
            this.scale = size_input.valueAsNumber || 1;
            size_output.value = Math.round(this.scale * 100) + "%";
            this.apply_scale();
        };
        size_input.onchange = () => this.save_state();

        document.getElementById("game_edit").onclick = (e) => {
            e.stopPropagation();
            this.set_editing(!this.editing);
        };

        // The joystick owns its pointer: with capture the finger may slide
        // outside the base and keeps steering until it is lifted.
        this.stick_base.addEventListener("pointerdown", (e) => {
            e.stopPropagation();
            if (this.stick_pointer !== null)
                return;
            this.stick_pointer = e.pointerId;
            this.stick_base.setPointerCapture(e.pointerId);
            if (this.editing) {
                // While editing the whole base is dragged around instead of
                // steering; remember the grab offset for a smooth follow.
                const base_rect = this.stick_base.getBoundingClientRect();
                this.stick_center_x = e.clientX - (base_rect.left + base_rect.width / 2);
                this.stick_center_y = e.clientY - (base_rect.top + base_rect.height / 2);
                return;
            }
            const rect = this.stick_base.getBoundingClientRect();
            this.stick_center_x = rect.left + rect.width / 2;
            this.stick_center_y = rect.top + rect.height / 2;
            this.stick_move(e);
        });
        this.stick_base.addEventListener("pointermove", (e) => {
            if (e.pointerId !== this.stick_pointer)
                return;
            if (this.editing) {
                this.drag_stick(e);
                return;
            }
            this.stick_move(e);
        });
        const stick_end = (e: PointerEvent) => {
            if (e.pointerId !== this.stick_pointer)
                return;
            this.stick_pointer = null;
            if (this.editing) {
                this.save_state();
                return;
            }
            this.stick_knob.style.transform = "translate(0px, 0px)";
            this.sync_stick_keys(new Set());
        };
        this.stick_base.addEventListener("pointerup", stick_end);
        this.stick_base.addEventListener("pointercancel", stick_end);

        // Any pointer starting on the overlay itself (controls stop
        // propagation) is a look gesture.
        this.overlay.addEventListener("pointerdown", (e) => {
            if (e.target !== this.overlay || this.look_pointer !== null)
                return;
            this.look_pointer = e.pointerId;
            this.look_last_x = e.clientX;
            this.look_last_y = e.clientY;
            this.look_rest_x = 0;
            this.look_rest_y = 0;
            this.overlay.setPointerCapture(e.pointerId);
        });
        this.overlay.addEventListener("pointermove", (e) => {
            if (e.pointerId !== this.look_pointer)
                return;
            const events = typeof e.getCoalescedEvents === "function" ? e.getCoalescedEvents() : [e];
            for (const ev of events)
                this.look_step(ev);
        });
        const look_end = (e: PointerEvent) => {
            if (e.pointerId === this.look_pointer)
                this.look_pointer = null;
        };
        this.overlay.addEventListener("pointerup", look_end);
        this.overlay.addEventListener("pointercancel", look_end);
        this.overlay.addEventListener("contextmenu", (e) => e.preventDefault());

        // The gear reuses the settings sheet toggle wired to the grabber
        // handle, keeping all open/close bookkeeping in one place.
        document.getElementById("game_settings").onclick = (e) => {
            e.stopPropagation();
            (document.getElementById("handle") as HTMLButtonElement).click();
        };

        // The quickbar is hidden in game mode, so the PC audio toggle gets
        // its own button here, left of the gear; the stream itself is shared
        // with the quickbar button (see wire_audio_button).
        const audioBtn = document.createElement("button");
        audioBtn.type = "button";
        audioBtn.className = "game_sysbtn";
        audioBtn.title = "PC audio on/off";
        audioBtn.setAttribute("aria-label", "Toggle PC audio");
        audioBtn.setAttribute("aria-pressed", "false");
        audioBtn.style.right = "calc(3.4rem + env(safe-area-inset-right))";
        audioBtn.innerHTML =
            '<svg viewBox="0 0 24 24" width="1.3em" height="1.3em" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
            '<path d="M11.25 4.75v14.5l-4.75-3.75H3.75A1 1 0 0 1 2.75 14.5v-5a1 1 0 0 1 1-1h2.75l4.75-3.75z"/>' +
            '<path d="M15.5 9a4.5 4.5 0 0 1 0 6"/><path d="M18.25 6.5a8.5 8.5 0 0 1 0 11"/></svg>';
        this.overlay.appendChild(audioBtn);
        audioBtn.addEventListener("pointerdown", (e) => e.stopPropagation());
        wire_audio_button(audioBtn);

        // Losing the page must never leave WASD or the fire button held
        // down on the host.
        document.addEventListener("visibilitychange", () => {
            if (document.hidden) {
                this.release_all();
            } else if (this.enabled) {
                this.request_wake_lock();
            }
        });

        const check = settings.checks.get("game_mode");
        check.onchange = (e) => {
            const enabled = (e.target as HTMLInputElement).checked;
            this.set_enabled(enabled);
            settings.save_settings();
            // The checkbox change doubles as the user gesture the fullscreen
            // API requires; landscape is the intended playing shape.
            if (enabled && !document.fullscreenElement && document.body.requestFullscreen) {
                document.body.requestFullscreen({ navigationUI: "hide" }).catch(() => { });
            }
        };
        // The checkbox was already restored from localStorage when settings
        // loaded, before this constructor ran: reapply the state.
        if (check.checked)
            this.set_enabled(true);
    }

    set_enabled(enabled: boolean) {
        this.enabled = enabled;
        if (!enabled)
            this.set_editing(false);
        this.release_all();
        this.overlay.classList.toggle("hide", !enabled);
        document.body.classList.toggle("game_mode", enabled);
        if (enabled) {
            this.request_wake_lock();
        } else if (this.wake_lock) {
            this.wake_lock.release();
            this.wake_lock = null;
        }
    }

    async request_wake_lock() {
        try {
            this.wake_lock = await (navigator as any).wakeLock.request("screen");
        } catch {
            // Denied or unsupported: the screen just sleeps as usual.
        }
    }

    // ------------------------------------------------------------ layout state

    load_state() {
        try {
            const raw = localStorage.getItem(GAME_LAYOUT_KEY);
            if (raw) {
                const state = JSON.parse(raw) as GameLayoutState;
                if (typeof state.preset === "string" && GAME_PRESETS[state.preset])
                    this.preset = state.preset;
                if (typeof state.scale === "number" && state.scale >= 0.5 && state.scale <= 2)
                    this.scale = state.scale;
                if (typeof state.stick?.x === "number" && typeof state.stick?.y === "number") {
                    this.stick_x = state.stick.x;
                    this.stick_y = state.stick.y;
                }
                for (const [id, buttons] of Object.entries(state.buttons ?? {}))
                    if (GAME_PRESETS[id] && Array.isArray(buttons))
                        this.layouts[id] = buttons;
            }
        } catch {
            log(LogLevel.DEBUG, "Failed to load the game layout.");
        }
        this.specs = clone_buttons(this.layouts[this.preset] ?? GAME_PRESETS[this.preset].buttons);
    }

    save_state() {
        this.layouts[this.preset] = clone_buttons(this.specs);
        const state: GameLayoutState = {
            preset: this.preset,
            scale: this.scale,
            stick: { x: this.stick_x ?? 16, y: this.stick_y ?? 76 },
            buttons: this.layouts,
        };
        try {
            localStorage.setItem(GAME_LAYOUT_KEY, JSON.stringify(state));
        } catch {
            // Storage full or disabled: the layout just does not persist.
        }
    }

    load_preset(id: string) {
        this.preset = id;
        this.specs = clone_buttons(this.layouts[id] ?? GAME_PRESETS[id].buttons);
        this.set_editing(false);
        this.render_buttons();
        this.save_state();
    }

    apply_scale() {
        this.overlay.style.setProperty("--game_scale", this.scale.toFixed(2));
    }

    apply_stick_position() {
        if (this.stick_x !== null)
            this.stick_base.style.left = this.stick_x + "%";
        if (this.stick_y !== null)
            this.stick_base.style.top = this.stick_y + "%";
    }

    drag_stick(e: PointerEvent) {
        const overlay_rect = this.overlay.getBoundingClientRect();
        const x = (e.clientX - overlay_rect.left - this.stick_center_x) / overlay_rect.width * 100;
        const y = (e.clientY - overlay_rect.top - this.stick_center_y) / overlay_rect.height * 100;
        this.stick_x = Math.min(95, Math.max(5, x));
        this.stick_y = Math.min(95, Math.max(8, y));
        this.stick_base.style.left = this.stick_x + "%";
        this.stick_base.style.top = this.stick_y + "%";
    }

    // Rebuild every action button from the current specs. Called after preset
    // switches, edits and deletions; system buttons and the joystick survive.
    render_buttons() {
        for (const el of Array.from(this.overlay.querySelectorAll(".game_btn")))
            el.remove();
        for (const spec of this.specs) {
            const btn = document.createElement("div");
            btn.id = spec.id;
            btn.classList.add("game_btn", spec.size);
            btn.textContent = spec.label;
            btn.style.left = spec.x + "%";
            btn.style.top = spec.y + "%";
            btn.addEventListener("pointerdown", (e) => {
                e.stopPropagation();
                btn.setPointerCapture(e.pointerId);
                if (this.editing) {
                    // Editing: tapping selects the button and starts a drag.
                    // The grab offset keeps the button from jumping under
                    // the finger.
                    this.select(spec.id);
                    const overlay_rect = this.overlay.getBoundingClientRect();
                    const rect = btn.getBoundingClientRect();
                    this.drag = {
                        id: spec.id,
                        off_x: (e.clientX - rect.left - rect.width / 2) / overlay_rect.width * 100,
                        off_y: (e.clientY - rect.top - rect.height / 2) / overlay_rect.height * 100,
                    };
                    return;
                }
                btn.classList.add("pressed");
                if (spec.mouse) {
                    this.press_mouse(spec.mouse);
                    if (spec.mouse == "left" && navigator.vibrate)
                        navigator.vibrate(12);
                } else if (spec.key) {
                    this.press_key(spec.key);
                }
            });
            btn.addEventListener("pointermove", (e) => {
                if (!this.editing || this.drag?.id !== spec.id)
                    return;
                const overlay_rect = this.overlay.getBoundingClientRect();
                const x = (e.clientX - overlay_rect.left) / overlay_rect.width * 100 - this.drag.off_x;
                const y = (e.clientY - overlay_rect.top) / overlay_rect.height * 100 - this.drag.off_y;
                spec.x = Math.min(98, Math.max(2, x));
                spec.y = Math.min(98, Math.max(2, y));
                btn.style.left = spec.x + "%";
                btn.style.top = spec.y + "%";
            });
            const release = (e: PointerEvent) => {
                if (this.editing) {
                    if (this.drag?.id === spec.id) {
                        this.drag = null;
                        this.save_state();
                    }
                    return;
                }
                if (spec.mouse) {
                    this.release_mouse(spec.mouse);
                } else if (spec.key) {
                    this.release_key(spec.key);
                }
                btn.classList.remove("pressed");
            };
            btn.addEventListener("pointerup", release);
            btn.addEventListener("pointercancel", release);
            if (spec.id === this.selected_id)
                btn.classList.add("selected");
            this.overlay.appendChild(btn);
        }
    }

    // ------------------------------------------------------------- edit mode

    selected_spec(): GameButtonSpec | undefined {
        return this.specs.find((s) => s.id === this.selected_id);
    }

    // Show the editor for `id` (or hide the panel with null).
    select(id: string | null) {
        this.selected_id = id;
        for (const el of Array.from(this.overlay.querySelectorAll(".game_btn.selected")))
            el.classList.remove("selected");
        if (!id) {
            this.sel_panel.classList.add("hide");
            return;
        }
        document.getElementById(id)?.classList.add("selected");
        const spec = this.selected_spec();
        if (!spec) {
            this.sel_panel.classList.add("hide");
            return;
        }
        this.label_input.value = spec.label;
        this.action_select.value = game_button_action(spec);
        this.size_select.value = spec.size;
        this.sel_panel.classList.remove("hide");
    }

    add_button() {
        const spec: GameButtonSpec = {
            id: "gb_u" + Date.now().toString(36),
            label: "BTN",
            key: "KeyF",
            x: 50,
            y: 50,
            size: "md",
        };
        this.specs.push(spec);
        this.render_buttons();
        this.select(spec.id);
        this.save_state();
    }

    delete_selected() {
        if (!this.selected_id)
            return;
        this.specs = this.specs.filter((s) => s.id !== this.selected_id);
        this.select(null);
        this.render_buttons();
        this.save_state();
    }

    // Drop the per-preset customization and fall back to the built-in layout.
    reset_layout() {
        delete this.layouts[this.preset];
        this.specs = clone_buttons(GAME_PRESETS[this.preset].buttons);
        this.select(null);
        this.render_buttons();
        this.save_state();
    }

    set_editing(on: boolean) {
        if (this.editing === on)
            return;
        this.editing = on;
        this.drag = null;
        // Leaving edit mode must not leave a held key behind.
        this.release_all();
        this.overlay.classList.toggle("edit", on);
        this.edit_bar.classList.toggle("hide", !on);
        if (!on)
            this.select(null);
        const edit_btn = document.getElementById("game_edit");
        if (edit_btn)
            edit_btn.textContent = on ? "Done editing" : "Edit layout";
    }

    // The bottom bar shown while editing: add / reset / done plus the editor
    // for the selected button (label, binding, size, delete).
    build_edit_ui() {
        this.edit_bar = document.createElement("div");
        this.edit_bar.id = "game_edit_bar";
        this.edit_bar.classList.add("hide");
        this.edit_bar.addEventListener("pointerdown", (e) => e.stopPropagation());

        const button = (text: string, cls: string, onclick: () => void) => {
            const b = document.createElement("button");
            b.type = "button";
            b.className = "ge_btn " + cls;
            b.textContent = text;
            b.onclick = (e) => {
                e.stopPropagation();
                onclick();
            };
            return b;
        };

        const row = document.createElement("div");
        row.className = "ge_row";
        row.appendChild(button("+ Add", "", () => this.add_button()));
        row.appendChild(button("Reset", "", () => this.reset_layout()));
        row.appendChild(button("Done", "ge_done", () => this.set_editing(false)));
        this.edit_bar.appendChild(row);

        this.sel_panel = document.createElement("div");
        this.sel_panel.className = "ge_row ge_panel hide";

        this.label_input = document.createElement("input");
        this.label_input.type = "text";
        this.label_input.className = "ge_input";
        this.label_input.maxLength = 6;
        this.label_input.placeholder = "Label";
        this.label_input.oninput = () => {
            const spec = this.selected_spec();
            if (!spec)
                return;
            spec.label = this.label_input.value;
            const el = document.getElementById(spec.id);
            if (el)
                el.textContent = spec.label;
            this.save_state();
        };

        this.action_select = document.createElement("select");
        this.action_select.className = "ge_select";
        const mouse_group = document.createElement("optgroup");
        mouse_group.label = "Mouse";
        for (const m of ["left", "right", "middle"]) {
            const opt = document.createElement("option");
            opt.value = "mouse:" + m;
            opt.textContent = "Mouse " + m;
            mouse_group.appendChild(opt);
        }
        const key_group = document.createElement("optgroup");
        key_group.label = "Key";
        for (const code of GAME_EDIT_KEYS) {
            const opt = document.createElement("option");
            opt.value = "key:" + code;
            opt.textContent = game_key_label(code);
            key_group.appendChild(opt);
        }
        this.action_select.appendChild(mouse_group);
        this.action_select.appendChild(key_group);
        this.action_select.onchange = () => {
            const spec = this.selected_spec();
            if (!spec)
                return;
            const value = this.action_select.value;
            if (value.startsWith("mouse:")) {
                spec.mouse = value.slice(6) as "left" | "middle" | "right";
                delete spec.key;
            } else {
                spec.key = value.slice(4);
                delete spec.mouse;
            }
            this.save_state();
        };

        this.size_select = document.createElement("select");
        this.size_select.className = "ge_select";
        for (const [value, label] of [["sm", "S"], ["md", "M"], ["lg", "L"]]) {
            const opt = document.createElement("option");
            opt.value = value;
            opt.textContent = label;
            this.size_select.appendChild(opt);
        }
        this.size_select.onchange = () => {
            const spec = this.selected_spec();
            if (!spec)
                return;
            spec.size = this.size_select.value as "sm" | "md" | "lg";
            const el = document.getElementById(spec.id);
            if (el) {
                el.classList.remove("sm", "md", "lg");
                el.classList.add(spec.size);
            }
            this.save_state();
        };

        this.sel_panel.appendChild(this.label_input);
        this.sel_panel.appendChild(this.action_select);
        this.sel_panel.appendChild(this.size_select);
        this.sel_panel.appendChild(button("\u2715", "ge_del", () => this.delete_selected()));
        this.edit_bar.appendChild(this.sel_panel);

        this.overlay.appendChild(this.edit_bar);
    }

    // Release every control state: on game mode changes and whenever the
    // page hides, so the host never keeps a stuck key or mouse button.
    release_all() {
        this.sync_stick_keys(new Set());
        for (const key of [...this.key_refs.keys()])
            this.send_key(key, false);
        this.key_refs.clear();
        for (const button of [...this.mouse_refs.keys()])
            this.send_click(button as "left" | "middle" | "right", false);
        this.mouse_refs.clear();
        this.look_pointer = null;
        this.stick_pointer = null;
        this.stick_knob.style.transform = "translate(0px, 0px)";
    }

    // Refcounted presses: the first holder sends the down event, the up
    // event goes out only when the last holder of that key/button leaves.
    press_key(code: string) {
        const n = (this.key_refs.get(code) ?? 0) + 1;
        this.key_refs.set(code, n);
        if (n === 1)
            this.send_key(code, true);
    }

    release_key(code: string) {
        const n = this.key_refs.get(code) ?? 0;
        if (n <= 1) {
            if (n === 1)
                this.send_key(code, false);
            this.key_refs.delete(code);
        } else {
            this.key_refs.set(code, n - 1);
        }
    }

    press_mouse(button: "left" | "middle" | "right") {
        const n = (this.mouse_refs.get(button) ?? 0) + 1;
        this.mouse_refs.set(button, n);
        if (n === 1)
            this.send_click(button, true);
    }

    release_mouse(button: "left" | "middle" | "right") {
        const n = this.mouse_refs.get(button) ?? 0;
        if (n <= 1) {
            if (n === 1)
                this.send_click(button, false);
            this.mouse_refs.delete(button);
        } else {
            this.mouse_refs.set(button, n - 1);
        }
    }

    // Bring the joystick's held WASD in line with `wanted`: press new keys,
    // release dropped ones.
    sync_stick_keys(wanted: Set<string>) {
        for (const key of [...this.stick_keys]) {
            if (!wanted.has(key)) {
                this.send_key(key, false);
                this.stick_keys.delete(key);
            }
        }
        for (const key of wanted) {
            if (!this.stick_keys.has(key)) {
                this.send_key(key, true);
                this.stick_keys.add(key);
            }
        }
    }

    stick_move(e: PointerEvent) {
        const rect = this.stick_base.getBoundingClientRect();
        const max = rect.width / 2;
        let dx = e.clientX - this.stick_center_x;
        let dy = e.clientY - this.stick_center_y;
        const dist = Math.hypot(dx, dy);
        if (dist > max) {
            dx = dx / dist * max;
            dy = dy / dist * max;
        }
        this.stick_knob.style.transform = "translate(" + dx + "px, " + dy + "px)";

        // Dead zone, then the stick vector picks a sector of the classic
        // 8-way WASD layout. Key codes, not characters: the host maps them
        // to virtual keys independent of its active layout.
        const wanted = new Set<string>();
        if (dist > max * 0.22) {
            const deg = Math.atan2(dy, dx) * 180 / Math.PI;
            if (deg > -157.5 && deg < -22.5)
                wanted.add("KeyW");
            if (deg > 22.5 && deg < 157.5)
                wanted.add("KeyS");
            if (Math.abs(deg) > 112.5)
                wanted.add("KeyA");
            if (Math.abs(deg) < 67.5)
                wanted.add("KeyD");
            // Analog speed, CS2 style: a gentle push past the dead zone
            // walks quietly (Shift held), pushing the stick out to the
            // edge releases Shift and runs at full speed.
            const travel = Math.min(dist / max, 1);
            if (travel < 0.55)
                wanted.add("ShiftLeft");
        }
        this.sync_stick_keys(wanted);
    }

    // All game mode traffic funnels through here: a dying websocket must
    // not throw while release_all is telling the host to let go.
    safe_send(payload: string) {
        if (this.webSocket.readyState === WebSocket.OPEN)
            this.webSocket.send(payload);
    }

    look_step(e: PointerEvent) {
        const sens = settings.game_sensitivity_input.valueAsNumber || 1;
        const dx = e.clientX - this.look_last_x;
        const dy = e.clientY - this.look_last_y;
        this.look_last_x = e.clientX;
        this.look_last_y = e.clientY;
        this.look_rest_x += dx * sens;
        this.look_rest_y += dy * sens;
        const send_x = Math.trunc(this.look_rest_x);
        const send_y = Math.trunc(this.look_rest_y);
        if (send_x === 0 && send_y === 0)
            return;
        this.look_rest_x -= send_x;
        this.look_rest_y -= send_y;
        this.safe_send(JSON.stringify({ "MouseMoveEvent": { dx: send_x, dy: send_y } }));
    }

    send_key(code: string, down: boolean) {
        const key = {
            event_type: down ? "down" : "up",
            code: code,
            key: code,
            location: 0,
            alt: false,
            ctrl: false,
            shift: false,
            meta: false,
        };
        this.safe_send(JSON.stringify({ "KeyboardEvent": key }));
    }

    send_click(button: "left" | "middle" | "right", down: boolean) {
        this.safe_send(JSON.stringify(
            { "ClickEvent": { "button": button, "action": down ? "down" : "up" } }
        ));
    }
}

// The stage overlay replaces the plain black screen before the first video
// frame arrives and turns into a readable error card if the websocket dies.
// Kept outside the connection machinery so it can be used during init(),
// before the settings object and the keyboard handlers exist.
class StageOverlay {
    root: HTMLElement;
    title: HTMLElement;
    sub: HTMLElement;
    hint: HTMLElement;
    retry: HTMLButtonElement;
    conn: HTMLOutputElement;
    socketReady: boolean;
    videoEnabled: boolean;
    videoPlaying: boolean;
    failed: boolean;
    failure: string;

    constructor() {
        this.root = document.getElementById("stage_overlay");
        this.title = document.getElementById("stage_title");
        this.sub = document.getElementById("stage_sub");
        this.hint = document.getElementById("stage_hint");
        this.retry = document.getElementById("stage_retry") as HTMLButtonElement;
        this.conn = document.getElementById("conn_state") as HTMLOutputElement;
        this.socketReady = false;
        this.videoEnabled = true;
        this.videoPlaying = false;
        this.failed = false;
        this.failure = "";
        this.retry.onclick = (e) => {
            e.stopPropagation();
            location.reload();
        };
        this.update();
    }

    set_video_enabled(enabled: boolean) {
        if (enabled && !this.videoEnabled) {
            // Wait for a fresh "playing" event after resuming the stream.
            this.videoPlaying = false;
        }
        this.videoEnabled = enabled;
        this.update();
    }

    on_socket_open() {
        this.socketReady = true;
        this.failed = false;
        this.failure = "";
        this.update();
    }

    on_socket_error(msg: string) {
        this.socketReady = false;
        this.failed = true;
        this.failure = msg;
        this.update();
    }

    on_playing() {
        this.videoPlaying = true;
        this.update();
    }

    private set_conn(label: string, state: string) {
        if (!this.conn)
            return;
        this.conn.textContent = label;
        this.conn.dataset.state = state;
    }

    private update() {
        // Page-level flag so CSS can trim/mute chrome while the stage card is
        // up (before the stream is live or while it is dead).
        let overlayHidden = false;

        if (this.failed) {
            this.root.dataset.state = "error";
            this.title.textContent = "Connection lost";
            this.sub.textContent = "The link to your PC closed.";
            this.hint.textContent = "Check that Proximity is still running on your computer, then reload this page.";
            this.retry.classList.remove("hide");
            this.set_conn("Offline", "error");
            this.root.classList.remove("hide");
            overlayHidden = false;
        }
        else if (!this.videoEnabled) {
            this.root.classList.add("hide");
            this.set_conn("Idle", "idle");
            overlayHidden = true;
        }
        else if (this.videoPlaying) {
            this.root.classList.add("hide");
            this.set_conn("Live", "live");
            overlayHidden = true;
        }
        else {
            this.retry.classList.add("hide");
            this.root.classList.remove("hide");
            this.root.dataset.state = this.socketReady ? "waiting" : "connecting";
            this.title.textContent = this.socketReady ? "Connected to your PC" : "Connecting to your PC";
            this.sub.textContent = this.socketReady ? "Waiting for the screen stream\u2026" : "Looking for the Proximity server\u2026";
            this.hint.textContent = this.socketReady
                ? "The screen will appear here in a moment."
                : "Keep Proximity running on your computer and both devices on the same network.";
            this.set_conn(this.socketReady ? "Connected" : "Connecting", this.socketReady ? "waiting" : "connecting");
            overlayHidden = false;
        }

        if (document.body) {
            document.body.classList.toggle("stage_live", overlayHidden);
        }
    }
}

function frame_rate_stats() {
    let t = performance.now();
    let fps = Math.round(frame_count / (t - last_fps_calc) * 10000) / 10;
    fps_out.value = fps.toString();
    frame_count = 0;
    last_fps_calc = t;
    setTimeout(() => frame_rate_stats(), 1500);
}

function handle_messages(
    webSocket: WebSocket,
    video: HTMLVideoElement,
    onConfigOk: Function,
    onConfigError: Function,
    onCapturableList: Function,
) {
    let mediaSource: MediaSource = null;
    let sourceBuffer: SourceBuffer = null;
    let media_url: string = null;
    let queue = [];
    const MAX_BUFFER_LENGTH = 20;  // In seconds
    function upd_buf() {
        if (sourceBuffer == null)
            return;
        if (!sourceBuffer.updating && queue.length > 0 && mediaSource.readyState == "open") {
            let buffer_length = 0;
            if (sourceBuffer.buffered.length) {
                // Assume only one time range...
                buffer_length = sourceBuffer.buffered.end(0) - sourceBuffer.buffered.start(0);
            }
            if (buffer_length > MAX_BUFFER_LENGTH) {
                sourceBuffer.remove(0, sourceBuffer.buffered.end(0) - MAX_BUFFER_LENGTH / 2);
                // This will trigger updateend when finished
            } else {
                try {
                    sourceBuffer.appendBuffer(queue.shift());
                } catch (err) {
                    log(LogLevel.DEBUG, "Error appending to sourceBuffer:" + err);
                    // Drop everything, and try to pick up the stream again
                    if (sourceBuffer.updating)
                        sourceBuffer.abort();
                    sourceBuffer.remove(0, Infinity);
                }
            }
        }
    }
    webSocket.onmessage = (event: MessageEvent) => {
        if (typeof event.data == "string") {
            let msg = JSON.parse(event.data);
            if (typeof msg == "string") {
                if (msg == "NewVideo") {
                    let MS = window.ManagedMediaSource ? window.ManagedMediaSource : window.MediaSource;
                    // The previous stream's blob URL is replaced here, so free
                    // it instead of leaking one per NewVideo (resolution
                    // changes, codec error recovery, reconnects).
                    if (media_url)
                        URL.revokeObjectURL(media_url);
                    mediaSource = new MS();
                    sourceBuffer = null;
                    media_url = URL.createObjectURL(mediaSource);
                    video.src = media_url;
                    mediaSource.addEventListener("sourceopen", (_) => {
                        let mimeType = 'video/mp4; codecs="avc1.4D403D"';
                        if (!MS.isTypeSupported(mimeType))
                            mimeType = "video/mp4";
                        sourceBuffer = mediaSource.addSourceBuffer(mimeType);
                        sourceBuffer.addEventListener("updateend", upd_buf);
                        // try to recover from errors by restarting the video
                        if (sourceBuffer.onerror)
                            sourceBuffer.onerror = () => settings.send_server_config();
                    })
                } else if (msg == "ConfigOk") {
                    onConfigOk();
                }
            } else if (typeof msg == "object") {
                if ("CapturableList" in msg)
                    onCapturableList(msg["CapturableList"]);
                else if ("Error" in msg)
                    alert(msg["Error"]);
                else if ("ConfigError" in msg) {
                    onConfigError(msg["ConfigError"]);
                } else if ("CustomInputAreas" in msg) {
                    settings.custom_input_areas = msg["CustomInputAreas"];
                    settings.checks.get("enable_custom_input_areas").checked = true;
                    settings.save_settings();
                }
            }

            return;
        }

        // not a string -> got a video frame
        queue.push(event.data);
        upd_buf();
        frame_count += 1;

        // only seek if there is data available, some browsers choke otherwise
        if (video.seekable.length > 0) {
            let seek_time = video.seekable.end(video.seekable.length - 1);
            if (video.readyState >= (settings.check_aggressive_seek.checked ? 3 : 4)
                // but make sure to catch up if the video is more than 3 seconds behind
                || seek_time - video.currentTime > 3) {
                if (isFinite(seek_time))
                    video.currentTime = seek_time;
                else
                    log(LogLevel.WARN, "Failed to seek to end of video.")
            }

        }
    }
}

function check_apis() {
    let apis = [
        {
            attrs: ["MediaSource", "ManagedMediaSource"],
            msg: "This browser doesn't support MSE/MMS required to playback video stream, try upgrading!"
        },
        {
            attrs: ["PointerEvent"],
            msg: "This browser doesn't support PointerEvents, input will not work, try upgrading!"
        },
    ];

    outer:
    for (let d of apis) {
        for (let attr of d.attrs) {
            if (attr in window) {
                continue outer;
            }
        }
        log(LogLevel.ERROR, d.msg);
    }
}

function init() {
    check_apis();

    let protocol = document.location.protocol == "https:" ? "wss://" : "ws://";
    let webSocket = new WebSocket(
        protocol + window.location.hostname + ":" +
        window.location.port + "/ws" + window.location.search
    );
    webSocket.binaryType = "arraybuffer";

    debug_overlay = document.getElementById("debug_overlay");
    settings = new Settings(webSocket);
    stage_overlay = new StageOverlay();
    stage_overlay.set_video_enabled(settings.video_enabled());

    for (const id of ["enable_video", "energysaving"]) {
        document.getElementById(id).addEventListener("change", () => {
            stage_overlay.set_video_enabled(settings.video_enabled());
        });
    }

    let video = document.getElementById("video") as HTMLVideoElement;
    let canvas = document.getElementById("canvas") as HTMLCanvasElement;
    video.addEventListener("playing", () => stage_overlay.on_playing());

    video.oncontextmenu = function(event) {
        event.preventDefault();
        event.stopPropagation();
        return false;
    };
    canvas.oncontextmenu = function(event) {
        event.preventDefault();
        event.stopPropagation();
        return false;
    };

    let toggle_fullscreen_btn = document.getElementById("fullscreen") as HTMLButtonElement;

    if (document.exitFullscreen) {
        toggle_fullscreen_btn.onclick = () => {
            if (!document.fullscreenElement)
                document.body.requestFullscreen({ navigationUI: "hide" });
            else
                document.exitFullscreen();
        }
    } else {
        // if document.exitFullscreen is not present we are probably running on iOS/iPadOS.
        // As input is broken in fullscreen mode on these, do not offer fullscreen in the first
        // place.
        toggle_fullscreen_btn.parentElement.removeChild(toggle_fullscreen_btn);
    }

    let handle_disconnect = (msg: string) => {
        // The game overlay may be holding WASD/fire on the host; the
        // release must not depend on the page being hidden.
        if (game_mode) {
            game_mode.release_all();
        }
        stage_overlay.on_socket_error(msg);
    }
    webSocket.onerror = () => handle_disconnect("Lost connection.");
    webSocket.onclose = () => handle_disconnect("Connection closed.");
    window.onresize = () => {
        stretch_video();
        canvas.width = window.innerWidth * window.devicePixelRatio;
        canvas.height = window.innerHeight * window.devicePixelRatio;
        let [w, h] = calc_max_video_resolution(settings.scale_video_input.valueAsNumber);
        settings.scale_video_output.value = w + "x" + h;
        settings.send_server_config();
    }
    video.controls = false;
    video.disableRemotePlayback = true;
    video.onloadeddata = () => stretch_video();
    let is_connected = false;
    handle_messages(webSocket, video, () => {
        if (!is_connected) {
            new KeyboardHandler(webSocket);
            new PointerHandler(webSocket);
            new SoftKeyboard(webSocket);
            new QuickBar(webSocket);
            game_mode = new GameMode(webSocket);
            is_connected = true;
        }
    },
        (err) => alert(err),
        (window_names) => settings.onCapturableList(window_names)
    );
    // Mobile browsers skip unload; pagehide is the event that reliably fires
    // when a phone tab is discarded mid-game. Release the game controls
    // before the socket goes away either way.
    const release_on_exit = () => {
        if (game_mode) {
            game_mode.release_all();
        }
        webSocket.close();
    };
    window.onunload = release_on_exit;
    window.addEventListener("pagehide", release_on_exit);
    webSocket.onopen = function(event) {
        stage_overlay.on_socket_open();
        webSocket.send('"GetCapturableList"');
        if (!settings.video_enabled())
            webSocket.send('"PauseVideo"');

        settings.send_server_config();

        document.onvisibilitychange = () => {
            if (document.hidden) {
                webSocket.send('"PauseVideo"');
            } else if (settings.video_enabled()) {
                webSocket.send('"ResumeVideo"');
            }
        };
    }
    frame_rate_stats();
}

// object-fit: fill; <-- this is unfortunately not supported on iOS, so we use the following
// workaround
function stretch_video() {
    let video = document.getElementById("video") as HTMLVideoElement;
    if (settings.stretched_video()) {
        video.style.transform = "scaleX(" + document.body.clientWidth / video.clientWidth + ") scaleY(" + document.body.clientHeight / video.clientHeight + ")";
    } else {
        let scale = Math.min(document.body.clientWidth / video.clientWidth, document.body.clientHeight / video.clientHeight);
        video.style.transform = "scale(" + scale + ")";
    }
}