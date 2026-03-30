import { useEffect, useState } from "react";
import { fetchDevices } from "../utils/deviceApi";

// Nueva función para obtener todos los drones registrados
async function fetchRegisteredDrones(): Promise<DeviceProfileType[]> {
  const res = await fetch("http://localhost:3002/api/drones");
  if (!res.ok)
    throw new Error("No se pudo obtener la lista de drones registrados");
  return res.json();
}
import RegisterForm from "./RegisterForm";
import DronProfileEditor from "./DronProfileEditor";
import type { DeviceProfile as DeviceProfileType } from "../types";

function Profile() {
  const [connectedDevices, setConnectedDevices] = useState<DeviceProfileType[]>(
    [],
  );
  const [registeredDrones, setRegisteredDrones] = useState<DeviceProfileType[]>(
    [],
  );
  const [deviceProfile, setDeviceProfile] = useState<DeviceProfileType | null>(
    null,
  );
  const [mensaje, setMensaje] = useState<string | null>(null);

  const LOGROS = [
    { nombre: "Primer vuelo", icono: "🚀" },
    { nombre: "10 vuelos", icono: "🎯" },
    { nombre: "Altura máxima", icono: "🏔️" },
  ];

  useEffect(() => {
    // Fetch conectados y registrados en paralelo
    Promise.all([fetchDevices(), fetchRegisteredDrones()])
      .then(([conectados, registrados]) => {
        setConnectedDevices(
          (conectados || []).filter((d) => d.id && d.id.trim() !== ""),
        );
        setRegisteredDrones(
          (registrados || []).filter((d) => d.id && d.id.trim() !== ""),
        );
      })
      .catch(() => setMensaje("Error al obtener dispositivos o drones"));
  }, []);

  useEffect(() => {
    if (deviceProfile) {
      setDeviceProfile({
        id: deviceProfile.id,
        nombre: `ESP32 ${deviceProfile.id}`,
        skin: "azul",
        conectado: true,
        ultimaConexion: new Date().toISOString(),
        registrado: false, // O true, según corresponda
      });
    } else {
      setDeviceProfile(null);
    }
  }, [deviceProfile]);

  // Drones conectados que NO están registrados (mostrar solo formulario de registro para estos)
  const conectadosNoRegistrados = connectedDevices.filter(
    (d) => !registeredDrones.some((r) => r.id === d.id),
  );

  // Todos los drones registrados (pueden estar o no conectados)
  const dronesRegistrados = registeredDrones;

  // Función placeholder para entrar al perfil
  const entrarAlPerfil = (id: string) => {
    // Aquí puedes navegar o mostrar el perfil
    alert(`Entrando al perfil de ${id}`);
  };

  return (
    <div className="flex flex-col items-center p-6">
      {/* Apartado 1: Drones conectados y registrados (pueden ingresar) */}
      <h2 className="text-lg font-bold mt-4 mb-2 text-green-400">
        Drones conectados
      </h2>
      <ul className="w-full max-w-md mb-4">
        {dronesRegistrados.filter((d) =>
          connectedDevices.some((cd) => cd.id === d.id),
        ).length === 0 && (
          <li className="text-gray-400">
            No hay drones conectados registrados
          </li>
        )}
        {dronesRegistrados
          .filter((d) => connectedDevices.some((cd) => cd.id === d.id))
          .map((d) => (
            <li
              key={d.id}
              className="flex items-center justify-between mb-2 p-2 bg-gray-800 rounded"
            >
              <span>{d.nombre || d.id}</span>
              <button
                className="px-2 py-1 rounded bg-blue-600 text-white hover:bg-blue-700"
                onClick={() => entrarAlPerfil(d.id)}
              >
                Ingresar
              </button>
              <span className="ml-2 px-2 py-0.5 rounded bg-green-700/70 text-green-200 text-xs font-mono tracking-widest shadow">
                Conectado
              </span>
            </li>
          ))}
      </ul>

      {/* Apartado 2: Drones conectados y NO registrados (pueden registrar) */}
      <h2 className="text-lg font-bold mt-4 mb-2 text-yellow-400">
        Drones conectados sin registrar
      </h2>
      <ul className="w-full max-w-md mb-4">
        {conectadosNoRegistrados.length === 0 && (
          <li className="text-gray-400">
            No hay drones conectados sin registrar
          </li>
        )}
        {conectadosNoRegistrados.map((d) => (
          <li
            key={d.id}
            className="flex items-center justify-between mb-2 p-2 bg-gray-800 rounded"
          >
            <span>{d.nombre || d.id}</span>
            <RegisterForm
              device={d}
              onRegistered={(nuevo: DeviceProfileType) => {
                setRegisteredDrones((prev: DeviceProfileType[]) => [
                  ...prev,
                  { ...d, ...nuevo },
                ]);
              }}
            />
          </li>
        ))}
      </ul>

      {/* Apartado 3: Drones registrados pero desconectados (no pueden ingresar) */}
      <h2 className="text-lg font-bold mt-4 mb-2 text-red-400">
        Drones registrados desconectados
      </h2>
      <ul className="w-full max-w-md">
        {dronesRegistrados.filter(
          (d) => !connectedDevices.some((cd) => cd.id === d.id),
        ).length === 0 && (
          <li className="text-gray-400">
            No hay drones registrados desconectados
          </li>
        )}
        {dronesRegistrados
          .filter((d) => !connectedDevices.some((cd) => cd.id === d.id))
          .map((d) => (
            <li
              key={d.id}
              className="flex items-center justify-between mb-2 p-2 bg-gray-800 rounded opacity-70"
            >
              <span>{d.nombre || d.id}</span>
              <span className="ml-2 px-2 py-0.5 rounded bg-red-700/70 text-red-200 text-xs font-mono tracking-widest shadow">
                Desconectado
              </span>
              {/* No mostrar botón de ingresar si está desconectado */}
            </li>
          ))}
      </ul>
      <div className="flex flex-col items-center mt-6">
        <svg width="112" height="112" viewBox="0 0 112 112">
          <circle
            cx="56"
            cy="56"
            r="50"
            stroke="#22d3ee"
            strokeWidth="8"
            fill="none"
            strokeDasharray={314}
            strokeDashoffset={314 * (1 - 0 / 100)}
            style={{ transition: "stroke-dashoffset 0.5s" }}
          />
          <text
            x="56"
            y="62"
            textAnchor="middle"
            fontSize="22"
            fill="#fff"
            fontWeight="bold"
          >
            0%
          </text>
        </svg>
        <div className="flex items-center gap-2 mt-4">
          <span className="font-bold text-lg text-white">
            {deviceProfile?.nombre || "Sin nombre"}
          </span>
          <span className="ml-2 px-2 py-0.5 rounded bg-green-700/70 text-green-200 text-xs font-mono tracking-widest shadow">
            Nivel 7
          </span>
        </div>
        <div className="flex items-center gap-2 mt-1">
          {LOGROS.map((l) => (
            <span
              key={l.nombre}
              title={l.nombre}
              className="text-xl select-none"
            >
              {l.icono}
            </span>
          ))}
        </div>
        {typeof mensaje === "string" && mensaje && (
          <div className="mt-2 text-sm text-yellow-200 bg-gray-800 px-3 py-1 rounded shadow">
            {mensaje}
          </div>
        )}
      </div>
      <div className="mt-6">
        {deviceProfile ? (
          <DronProfileEditor
            deviceProfile={deviceProfile}
            onProfileUpdate={async (nuevoNombre: string) => {
              try {
                const res = await fetch(
                  `http://localhost:3002/api/drones/${deviceProfile.id}`,
                  {
                    method: "PUT",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify({
                      ...deviceProfile,
                      nombre: nuevoNombre,
                    }),
                  },
                );
                if (!res.ok) throw new Error("No se pudo actualizar el nombre");
                const actualizado = await res.json();
                setDeviceProfile(actualizado);
                setRegisteredDrones((prev: DeviceProfileType[]) =>
                  prev.map((d: DeviceProfileType) =>
                    d.id === actualizado.id
                      ? { ...d, nombre: actualizado.nombre }
                      : d,
                  ),
                );
                setMensaje("Nombre actualizado correctamente");
              } catch {
                setMensaje("Error al actualizar el nombre");
              }
              setTimeout(() => setMensaje(null), 1500);
            }}
          />
        ) : (
          <div className="text-gray-400 p-6 text-lg">
            Selecciona un ESP32 conectado para ver su perfil.
          </div>
        )}
      </div>
    </div>
  );
}

export default Profile;
