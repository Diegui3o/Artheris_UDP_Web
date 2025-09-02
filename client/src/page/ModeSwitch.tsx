import { useCallback, useEffect, useMemo, useRef, useState } from "react";

const LS_KEY = "armeris:mode";
type ModeNum = 0 | 1 | 2;

type MsgMode = { type: "mode"; value: number | string };
type MsgModo = { type: "modo"; value: number | string };
type MsgAck = {
  type: "ack";
  ok: boolean;
  info?: string;
  boot_id?: string | number;
  bootId?: string | number;
};
type MsgCurMode = { type: "current_mode"; mode: number | string };
type MsgModeUpd = { type: "mode_update"; mode: number | string };
type MsgSnapshot = {
  type: "snapshot";
  mode?: number | string;
  boot_id?: string | number;
  uptime_ms?: number;
};
type MsgStatus = {
  type: "status";
  modoActual?: number | string;
  mode?: number | string;
  boot_id?: string | number;
  uptime_ms?: number;
};
type IncomingWS =
  | MsgMode
  | MsgModo
  | MsgAck
  | MsgCurMode
  | MsgModeUpd
  | MsgSnapshot
  | MsgStatus
  | Record<string, unknown>;

const MODE_LABEL: Record<ModeNum, string> = {
  0: "Piloto",
  1: "Espera",
  2: "Manual",
};

export default function ModeSwitch() {
  const [modo, setModo] = useState<ModeNum | null>(null);
  const [ws, setWs] = useState<WebSocket | null>(null);

  // modo pendiente de confirmar por ACK
  const pendingModeRef = useRef<ModeNum | null>(null);

  // ====== Normalizador de modo ======
  const normalizeMode = useMemo(
    () =>
      (raw: unknown): ModeNum | null => {
        if (raw === undefined || raw === null) return null;

        // Handle numbers directly
        if (typeof raw === "number") {
          // Ensure we handle 0 correctly (pilot mode)
          if (raw === 0) return 0;
          if (raw === 1) return 1;
          if (raw === 2 || raw === 3) return 2; // Map 3 to 2 (manual)
          return null;
        }

        // Handle strings
        if (typeof raw === "string") {
          const s = raw.trim().toLowerCase();
          if (s === "pilot" || s === "piloto" || s === "0") return 0;
          if (s === "idle" || s === "espera" || s === "1") return 1;
          if (s === "manual" || s === "2" || s === "3") return 2;
          const n = Number(s);
          if (!isNaN(n)) {
            if (n === 0) return 0;
            if (n === 1) return 1;
            if (n === 2 || n === 3) return 2;
          }
        }

        return null;
      },
    []
  );

  // ====== Autoridad (boot/uptime) ======
  const lastRef = useRef<{
    bootId?: string | number;
    uptime?: number;
    ts?: number;
    mode?: ModeNum | null;
  }>({});

  // Track last received mode from server
  const lastServerModeRef = useRef<number | null>(null);

  const isNewer = (bootId?: string | number, uptime?: number) => {
    const L = lastRef.current;
    if (bootId && L.bootId && bootId !== L.bootId) return true; // boot nuevo gana
    if (bootId && L.bootId && bootId === L.bootId) {
      if (typeof uptime === "number" && typeof L.uptime === "number")
        return uptime > L.uptime;
    }
    if (!L.ts) return true; // primera vez
    return true; // por defecto aceptamos
  };

  const stampAndApply = (
    mode: ModeNum | null,
    bootId?: string | number,
    uptime?: number
  ) => {
    if (mode !== null) {
      // Always update the mode from server, but log if it's different
      const modeChanged = lastServerModeRef.current !== mode;
      if (modeChanged) {
        console.log(
          "Modo actualizado desde servidor:",
          mode,
          "(anterior:",
          lastServerModeRef.current,
          ")"
        );
        lastServerModeRef.current = mode;
        setModo(mode);
        localStorage.setItem(LS_KEY, String(mode));
      }
    }

    lastRef.current = {
      bootId: bootId ?? lastRef.current.bootId,
      uptime: uptime ?? lastRef.current.uptime,
      ts: Date.now(),
      mode,
    };

    // Clear pending mode if it matches the current mode
    if (pendingModeRef.current === mode) {
      pendingModeRef.current = null;
    }
  };

  type ModeObject = {
    boot_id?: string | number;
    bootId?: string | number;
    uptime_ms?: number;
    uptime?: number;
    mode?: unknown;
    modo?: unknown;
    modoActual?: unknown;
    current_mode?: unknown;
    payload?: {
      mode?: unknown;
      modo?: unknown;
      modoActual?: unknown;
    };
    angles?: {
      mode?: unknown;
      modo?: unknown;
      modoActual?: unknown;
    };
    data?: {
      mode?: unknown;
      modo?: unknown;
      modoActual?: unknown;
    };
    [key: string]: unknown;
  };

  // ====== Placeholder inicial desde localStorage (UNA sola vez, dentro del componente) ======
  useEffect(() => {
    // Don't set initial mode from localStorage - wait for server state
    // This prevents showing stale data while we wait for the current mode from the server

    // Only use localStorage as a fallback if we don't get a response from the server
    const fallbackTimeout = setTimeout(() => {
      if (modo === null) {
        const saved = localStorage.getItem(LS_KEY);
        if (saved !== null) {
          const n = Number(saved);
          if ([0, 1, 2].includes(n)) {
            console.log("Usando modo guardado localmente:", n);
            setModo(n as ModeNum);
          }
        }
      }
    }, 2000); // Wait 2 seconds for server response before using localStorage

    return () => clearTimeout(fallbackTimeout);
  }, [modo]);

  // ====== WebSocket con reconexión + uso de applyAuthoritative ======
  const applyAuthoritative = useCallback(
    (obj: unknown): boolean => {
      if (!obj || typeof obj !== "object") return false;
      const modeObj = obj as ModeObject;

      const bootId = "boot_id" in modeObj ? modeObj.boot_id : modeObj.bootId;
      const uptime =
        "uptime_ms" in modeObj && typeof modeObj.uptime_ms === "number"
          ? modeObj.uptime_ms
          : "uptime" in modeObj && typeof modeObj.uptime === "number"
          ? modeObj.uptime
          : undefined;

      // Si detecto boot distinto, invalidar UI y limpiar "modo pendiente"
      if (
        bootId &&
        lastRef.current.bootId &&
        bootId !== lastRef.current.bootId
      ) {
        setModo(null);
        pendingModeRef.current = null;
      }

      // Candidatos en distintos niveles
      const candidates: unknown[] = [
        "mode" in modeObj ? modeObj.mode : undefined,
        "modo" in modeObj ? modeObj.modo : undefined,
        "modoActual" in modeObj ? modeObj.modoActual : undefined,
        "current_mode" in modeObj ? modeObj.current_mode : undefined,
        "payload" in modeObj &&
        modeObj.payload &&
        typeof modeObj.payload === "object" &&
        "mode" in modeObj.payload
          ? modeObj.payload.mode
          : undefined,
        "payload" in modeObj &&
        modeObj.payload &&
        typeof modeObj.payload === "object" &&
        "modo" in modeObj.payload
          ? modeObj.payload.modo
          : undefined,
        "payload" in modeObj &&
        modeObj.payload &&
        typeof modeObj.payload === "object" &&
        "modoActual" in modeObj.payload
          ? modeObj.payload.modoActual
          : undefined,
        "angles" in modeObj &&
        modeObj.angles &&
        typeof modeObj.angles === "object" &&
        "mode" in modeObj.angles
          ? modeObj.angles.mode
          : undefined,
        "angles" in modeObj &&
        modeObj.angles &&
        typeof modeObj.angles === "object" &&
        "modo" in modeObj.angles
          ? modeObj.angles.modo
          : undefined,
        "angles" in modeObj &&
        modeObj.angles &&
        typeof modeObj.angles === "object" &&
        "modoActual" in modeObj.angles
          ? modeObj.angles.modoActual
          : undefined,
        "data" in modeObj &&
        modeObj.data &&
        typeof modeObj.data === "object" &&
        "mode" in modeObj.data
          ? modeObj.data.mode
          : undefined,
        "data" in modeObj &&
        modeObj.data &&
        typeof modeObj.data === "object" &&
        "modo" in modeObj.data
          ? modeObj.data.modo
          : undefined,
        "data" in modeObj &&
        modeObj.data &&
        typeof modeObj.data === "object" &&
        "modoActual" in modeObj.data
          ? modeObj.data.modoActual
          : undefined,
      ].filter(Boolean);

      for (const c of candidates) {
        const v = normalizeMode(c);
        if (v !== null && isNewer(bootId, uptime)) {
          stampAndApply(v, bootId, uptime);
          return true;
        }
      }
      return false;
    },
    [normalizeMode]
  );

  useEffect(() => {
    let closedByUser = false;
    let attempt = 0;
    let sock: WebSocket;

    const connect = () => {
      sock = new WebSocket("ws://localhost:9001");
      setWs(sock);

      sock.onopen = () => {
        attempt = 0;
        console.log("✅ Conectado (modo)");
        try {
          sock.send(JSON.stringify({ type: "get_mode" }));
          sock.send(JSON.stringify({ type: "get_snapshot" }));
        } catch {
          /* no-op */
        }
      };

      sock.onmessage = (ev) => {
        let data: unknown;
        try {
          data = JSON.parse(ev.data);
        } catch (e) {
          console.warn("WS: JSON inválido", e);
          return;
        }
        const msg = data as IncomingWS;

        // 1) Mensajes sin type (ya lo tienes)
        if (!msg || typeof msg !== "object") return;

        // Handle messages with modoActual directly
        if ("modoActual" in msg) {
          const mode = normalizeMode(msg.modoActual);
          if (mode !== null) {
            stampAndApply(mode);
            return;
          }
        }

        // Handle other message formats
        if (!("type" in msg)) {
          let ok = applyAuthoritative(msg);
          if (!ok) {
            if (Array.isArray(msg)) {
              for (const it of msg) {
                ok = applyAuthoritative(it);
                if (ok) break;
              }
            } else if (
              typeof msg === "object" &&
              msg !== null &&
              "angles" in msg &&
              typeof msg.angles === "object" &&
              msg.angles !== null
            ) {
              applyAuthoritative(msg.angles);
            }
          }
          return;
        }

        // 2) Mensajes con type: intenta también con anidados (angles/data) ANTES del switch
        if (typeof msg === "object" && msg !== null) {
          const msgObj = msg as Record<string, unknown>;
          if (
            "angles" in msgObj &&
            typeof msgObj.angles === "object" &&
            msgObj.angles !== null
          ) {
            if (applyAuthoritative(msgObj.angles)) return;
          }
          if (
            "data" in msgObj &&
            typeof msgObj.data === "object" &&
            msgObj.data !== null
          ) {
            if (applyAuthoritative(msgObj.data)) return;
          }
        }

        // 2.b) Con "type"
        const msgType =
          "type" in msg && typeof msg.type === "string" ? msg.type : "";
        switch (msgType) {
          case "snapshot": {
            if (!applyAuthoritative(msg)) {
              const m = (msg as MsgSnapshot).mode;
              if (m !== undefined) applyAuthoritative({ mode: m });
            }
            return;
          }
          case "status": {
            if (!applyAuthoritative(msg)) {
              const st = msg as MsgStatus;
              if (st.modoActual !== undefined)
                applyAuthoritative({ mode: st.modoActual, ...st });
              else if (st.mode !== undefined)
                applyAuthoritative({ mode: st.mode, ...st });
            }
            return;
          }
          case "mode": {
            const m = (msg as MsgMode).value;
            applyAuthoritative({ mode: m, ...msg });
            return;
          }
          case "modo": {
            const m = (msg as MsgModo).value;
            applyAuthoritative({ mode: m, ...msg });
            return;
          }
          case "current_mode": {
            const m = (msg as MsgCurMode).mode;
            applyAuthoritative({ mode: m, ...msg });
            return;
          }
          case "mode_update": {
            const m = (msg as MsgModeUpd).mode;
            applyAuthoritative({ mode: m, ...msg });
            return;
          }
          case "ack": {
            const ack = msg as MsgAck;
            const ackBoot = ack?.boot_id ?? ack?.bootId;

            // 1) "info":"mode->2"
            const info = ack.info ?? "";
            if (typeof info === "string" && info.startsWith("mode->")) {
              const m = Number(info.split("->")[1]);
              applyAuthoritative({ mode: m, boot_id: ackBoot });
              return;
            }

            // 2) ack simple
            if (ack.ok && pendingModeRef.current !== null) {
              applyAuthoritative({
                mode: pendingModeRef.current,
                boot_id: ackBoot,
              });
            }
            return;
          }
          default: {
            applyAuthoritative(msg);
            return;
          }
        }
      };

      sock.onerror = (e) => console.error("❌ WS modo error:", e);

      sock.onclose = () => {
        console.log("🔌 WS cerrado");
        setWs(null);
        // invalida UI y autoridad
        setModo(null);
        lastRef.current = {};
        if (!closedByUser) {
          attempt += 1;
          const delay = Math.min(30000, 500 * 2 ** attempt);
          setTimeout(connect, delay);
        }
      };
    };

    connect();

    return () => {
      closedByUser = true;
      if (sock && sock.readyState === WebSocket.OPEN) sock.close();
    };
  }, [normalizeMode, applyAuthoritative]);

  // ====== Envío de comando ======
  const cambiarModo = (nuevo: ModeNum) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WS no conectado");
      return;
    }

    // Don't clear pending mode here - we want to track it until we get a response
    pendingModeRef.current = nuevo;

    // Send the command to change the mode
    const command = {
      type: "command",
      payload: { mode: nuevo },
      request_id: Date.now().toString(), // Add request_id for tracking
    };

    // Update UI optimistically for better UX
    setModo(nuevo);
    localStorage.setItem(LS_KEY, String(nuevo));

    ws.send(JSON.stringify(command));
    console.log(`Solicitando modo -> ${nuevo}`, command);
  };

  // Define mode configurations
  const modes = [
    { id: 0, name: "Piloto", color: "from-blue-500 to-blue-600", icon: "✈️" },
    {
      id: 1,
      name: "Espera",
      color: "from-yellow-500 to-yellow-600",
      icon: "⏳",
    },
    {
      id: 2,
      name: "Manual",
      color: "from-purple-500 to-purple-600",
      icon: "✋",
    },
  ] as const;

  // Check if we're in a loading state
  const isLoading = modo === null;
  const isConnected = ws && ws.readyState === WebSocket.OPEN;

  return (
    <div className="bg-gray-800/80 backdrop-blur-sm p-6 rounded-2xl shadow-xl border border-gray-700/50 max-w-md w-full">
      <div className="flex items-center justify-between mb-6">
        <div>
          <h2 className="text-xl font-bold text-white">Modo de Operación</h2>
          <p className="text-gray-400 text-sm">
            {isConnected ? "Conectado al servidor" : "Desconectado"}
          </p>
        </div>
        <div className="flex items-center space-x-2">
          <div
            className={`w-3 h-3 rounded-full ${
              isConnected ? "bg-green-500" : "bg-red-500"
            }`}
          ></div>
          <span className="text-sm text-gray-400">
            {isConnected ? "En línea" : "Offline"}
          </span>
        </div>
      </div>

      {isLoading ? (
        <div className="flex justify-center items-center py-8">
          <div className="animate-spin rounded-full h-8 w-8 border-t-2 border-b-2 border-cyan-500"></div>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="bg-gray-900/50 p-4 rounded-xl border border-gray-700/50 mb-4">
            <p className="text-sm text-gray-400 mb-1">Modo actual</p>
            <div className="flex items-center">
              <div className="w-10 h-10 rounded-lg bg-gradient-to-br from-cyan-500 to-blue-600 flex items-center justify-center text-white text-xl mr-3">
                {modes[modo]?.icon || "?"}
              </div>
              <div>
                <h3 className="text-white font-medium">{MODE_LABEL[modo]}</h3>
                <p className="text-xs text-gray-400">ID: {modo}</p>
              </div>
            </div>
          </div>

          <div className="grid grid-cols-3 gap-3">
            {modes.map((mode) => (
              <button
                key={mode.id}
                onClick={() => cambiarModo(mode.id as ModeNum)}
                disabled={!isConnected}
                className={`relative p-4 rounded-xl transition-all duration-300 transform hover:scale-105 ${
                  modo === mode.id
                    ? `bg-gradient-to-br ${mode.color} text-white shadow-lg`
                    : "bg-gray-800 hover:bg-gray-700 text-gray-300"
                } border ${
                  modo === mode.id ? "border-cyan-500/50" : "border-gray-700"
                } flex flex-col items-center justify-center h-24`}
              >
                <span className="text-2xl mb-1">{mode.icon}</span>
                <span className="text-sm font-medium">{mode.name}</span>
                {modo === mode.id && (
                  <div className="absolute -top-2 -right-2 w-4 h-4 bg-green-500 rounded-full border-2 border-gray-800"></div>
                )}
              </button>
            ))}
          </div>
        </div>
      )}

      <div className="mt-6 pt-4 border-t border-gray-700/50">
        <div className="flex items-center justify-between text-xs text-gray-500">
          <span>Estado: {isConnected ? "Conectado" : "Desconectado"}</span>
          <span>v1.0.0</span>
        </div>
      </div>
    </div>
  );
}
