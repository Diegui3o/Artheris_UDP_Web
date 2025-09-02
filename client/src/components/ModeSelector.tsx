import { useCallback, useEffect, useRef, useState } from "react";

const MODES = [
  { id: 0, name: "Piloto", color: "from-blue-500 to-blue-600" },
  { id: 1, name: "Espera", color: "from-yellow-500 to-yellow-600" },
  { id: 2, name: "Manual", color: "from-green-500 to-green-600" },
];

interface ModeSelectorProps {
  className?: string;
  onModeChange?: (mode: number | null) => void;
  onConnectionStatusChange?: (
    status: "connecting" | "connected" | "disconnected"
  ) => void;
}

type ModeNum = 0 | 1 | 2 | 3;

export default function ModeSelector({
  className = "",
  onModeChange,
  onConnectionStatusChange,
}: ModeSelectorProps) {
  const [activeMode, setActiveMode] = useState<ModeNum | null>(null);
  const [ws, setWs] = useState<WebSocket | null>(null);
  const pendingModeRef = useRef<ModeNum | null>(null);
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const lastPingRef = useRef<number>(0);
  const pingIntervalRef = useRef<NodeJS.Timeout | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<
    "connecting" | "connected" | "disconnected"
  >("connecting");

  // Notify parent of connection status changes
  useEffect(() => {
    onConnectionStatusChange?.(connectionStatus);
  }, [connectionStatus, onConnectionStatusChange]);

  // Normalize mode from various input types
  const normalizeMode = useCallback((raw: unknown): ModeNum | null => {
    if (raw === undefined || raw === null) return null;

    if (typeof raw === "number") {
      if (raw === 0) return 0;
      if (raw === 1) return 1;
      if (raw === 2 || raw === 3) return 2;
      return null;
    }

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
  }, []);

  // Handle WebSocket messages
  const handleMessage = useCallback(
    (event: MessageEvent) => {
      try {
        const data = JSON.parse(event.data as string) as {
          type: string;
          mode?: unknown;
          modoActual?: unknown;
        };

        if (data.type === "current_mode" || data.type === "mode_update") {
          const mode = normalizeMode(data.mode);
          if (mode !== null) {
            setActiveMode(mode);
            onModeChange?.(mode);
            pendingModeRef.current = null;
          }
        } else if (
          data.type === "status" &&
          (data.modoActual !== undefined || data.mode !== undefined)
        ) {
          const mode = normalizeMode(data.modoActual ?? data.mode);
          if (mode !== null) {
            setActiveMode(mode);
            onModeChange?.(mode);
            pendingModeRef.current = null;
          }
        } else if (data.type === "pong") {
          lastPingRef.current = Date.now();
        }
      } catch (e) {
        console.error("Error parsing WebSocket message:", e);
      }
    },
    [normalizeMode, onModeChange]
  );

  // Initialize WebSocket connection
  useEffect(() => {
    let socket: WebSocket | null = null;
    let isMounted = true;
    let reconnectAttempts = 0;
    const MAX_RECONNECT_ATTEMPTS = 5;

    const connect = () => {
      if (!isMounted) return;

      // Don't create a new connection if one already exists and is in a connecting/connected state
      if (
        socket &&
        (socket.readyState === WebSocket.CONNECTING ||
          socket.readyState === WebSocket.OPEN)
      ) {
        return;
      }

      console.log("Attempting to connect to WebSocket...");
      socket = new WebSocket("ws://localhost:9001/ws");

      const onOpen = () => {
        if (!isMounted || !socket) return;

        console.log("WebSocket connected");
        setWs(socket);
        setConnectionStatus("connected");
        reconnectAttempts = 0; // Reset reconnect attempts on successful connection

        // Clear any pending reconnection
        if (reconnectTimeoutRef.current) {
          clearTimeout(reconnectTimeoutRef.current);
          reconnectTimeoutRef.current = null;
        }

        // Request current mode
        try {
          socket.send(JSON.stringify({ type: "get_mode" }));
        } catch (e) {
          console.error("Error sending get_mode:", e);
        }

        // Start ping interval
        if (pingIntervalRef.current) {
          clearInterval(pingIntervalRef.current);
        }

        pingIntervalRef.current = setInterval(() => {
          if (socket && socket.readyState === WebSocket.OPEN) {
            try {
              socket.send(JSON.stringify({ type: "ping" }));
              lastPingRef.current = Date.now();
            } catch (e) {
              console.error("Error sending ping:", e);
            }
          }
        }, 15000); // Ping every 15 seconds
      };

      const onMessage = (event: MessageEvent) => {
        if (isMounted) {
          handleMessage(event);
        }
      };

      const onClose = (event: CloseEvent) => {
        if (!isMounted) return;

        console.log("WebSocket disconnected", event.code, event.reason);
        if (socket === ws) {
          setWs(null);
        }
        setConnectionStatus("disconnected");

        // Clean up ping interval
        if (pingIntervalRef.current) {
          clearInterval(pingIntervalRef.current);
          pingIntervalRef.current = null;
        }

        // Only attempt to reconnect if the close was not initiated by the client
        // and we haven't exceeded max reconnection attempts
        if (event.code !== 1000 && reconnectAttempts < MAX_RECONNECT_ATTEMPTS) {
          reconnectAttempts++;
          console.log(
            `Scheduling reconnection attempt ${reconnectAttempts}/${MAX_RECONNECT_ATTEMPTS}...`
          );
          setConnectionStatus("connecting");

          // Exponential backoff with jitter
          const baseDelay = Math.min(
            30000,
            1000 * Math.pow(2, reconnectAttempts)
          );
          const jitter = Math.random() * 1000;
          const delay = baseDelay + jitter;

          reconnectTimeoutRef.current = setTimeout(() => {
            if (isMounted) {
              reconnectTimeoutRef.current = null;
              connect();
            }
          }, delay);
        } else if (reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
          console.log("Max reconnection attempts reached");
        }
      };

      const onError = (error: Event) => {
        console.error("WebSocket error:", error);
        // Let onclose handle reconnection
        if (socket) {
          socket.close();
        }
      };

      socket.onopen = onOpen;
      socket.onmessage = onMessage;
      socket.onclose = onClose;
      socket.onerror = onError;
    };

    // Initial connection
    connect();

    return () => {
      isMounted = false;

      // Clean up WebSocket
      if (socket) {
        try {
          const socketToClose = socket;
          socket = null;

          if (
            socketToClose.readyState === WebSocket.OPEN ||
            socketToClose.readyState === WebSocket.CONNECTING
          ) {
            socketToClose.close(1000, "Component unmounting");
          }
        } catch (e) {
          console.error("Error closing WebSocket:", e);
        }
      }

      // Clean up intervals and timeouts
      if (pingIntervalRef.current) {
        clearInterval(pingIntervalRef.current);
        pingIntervalRef.current = null;
      }
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
        reconnectTimeoutRef.current = null;
      }

      // Reset state if unmounting
      if (isMounted) {
        setWs(null);
        setConnectionStatus("disconnected");
      }
    };
  }, [handleMessage, ws]); // Removed ws from dependencies to prevent reconnection loops

  const handleModeChange = (modeId: ModeNum) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;

    // Optimistic UI update
    setActiveMode(modeId);
    pendingModeRef.current = modeId;

    // Send mode change request
    ws.send(
      JSON.stringify({
        type: "set_mode",
        mode: modeId,
        timestamp: Date.now(),
      })
    );

    // Timeout in case we don't get a response
    setTimeout(() => {
      if (pendingModeRef.current === modeId) {
        pendingModeRef.current = null;
      }
    }, 1000);
  };

  // Calculate indicator position safely
  const getIndicatorPosition = () => {
    if (activeMode === null) return { width: "0", left: "0" };
    return {
      width: `calc(${100 / MODES.length}% - 0.5rem)`,
      left: `calc(${
        (activeMode / (MODES.length - 1)) *
        ((MODES.length - 1) / MODES.length) *
        100
      }% + 0.25rem)`,
    };
  };

  const indicatorPosition = getIndicatorPosition();
  const isDisabled = connectionStatus !== "connected";

  return (
    <div className={`relative ${className}`}>
      <div className="relative z-10 flex rounded-lg bg-gray-800 p-1">
        {MODES.map((mode) => (
          <button
            key={mode.id}
            onClick={() => !isDisabled && handleModeChange(mode.id as ModeNum)}
            disabled={isDisabled}
            className={`relative z-10 flex-1 py-2.5 px-4 text-sm font-medium transition-colors duration-200
              ${
                activeMode === mode.id
                  ? "text-white"
                  : "text-gray-400 hover:text-white"
              }
              ${
                isDisabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer"
              }`}
          >
            {mode.name}
          </button>
        ))}

        {/* Active indicator with gradient */}
        {activeMode !== null && (
          <div
            className="absolute inset-y-1 left-1 z-0 rounded-md transition-all duration-300 ease-out"
            style={{
              ...indicatorPosition,
              background: `linear-gradient(135deg, ${modeToGradient(
                activeMode
              )})`,
              opacity: isDisabled ? 0.5 : 1,
            }}
          />
        )}
      </div>

      {/* Progress bar */}
      <div className="mt-4 h-1.5 rounded-full bg-gray-800 overflow-hidden">
        <div
          className="h-full transition-all duration-500 ease-out"
          style={{
            width:
              activeMode !== null
                ? `${((activeMode + 1) / MODES.length) * 100}%`
                : "0%",
            background: `linear-gradient(90deg, ${modeToGradient(
              0
            )}, ${modeToGradient(1)}, ${modeToGradient(2)})`,
            opacity: isDisabled ? 0.5 : 1,
          }}
        />
      </div>

      {/* Connection status indicator */}
      <div className="flex items-center justify-end mt-2">
        <div className="flex items-center text-xs text-gray-400">
          <span
            className={`inline-block w-2 h-2 rounded-full mr-1 ${
              connectionStatus === "connected"
                ? "bg-green-500"
                : connectionStatus === "connecting"
                ? "bg-yellow-500"
                : "bg-red-500"
            }`}
          ></span>
          {connectionStatus === "connected"
            ? "Conectado"
            : connectionStatus === "connecting"
            ? "Conectando..."
            : "Desconectado"}
        </div>
      </div>
    </div>
  );
}

// Helper function to get gradient based on mode
function modeToGradient(mode: ModeNum | null): string {
  if (mode === null) return "#6b7280, #9ca3af"; // Gray for disconnected
  switch (mode) {
    case 0:
      return "#3b82f6, #60a5fa"; // Blue for Piloto
    case 1:
      return "#eab308, #facc15"; // Yellow for Espera
    case 2:
      return "#22c55e, #4ade80"; // Green for Manual
    default:
      return "#6b7280, #9ca3af"; // Default gray
  }
}
