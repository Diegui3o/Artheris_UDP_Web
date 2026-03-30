"use client";

interface MetricsTableProps {
  data: any;
}

export default function MetricsTable({ data }: MetricsTableProps) {
  if (!data) return null;

  const { error_metrics, comparison_roll, comparison_pitch } = data;

  return (
    <div className="space-y-6">
      {/* Error Metrics */}
      <div>
        <h3 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
          <span className="w-2 h-2 bg-red-500 rounded-full"></span>
          Error de Seguimiento
        </h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div className="bg-gray-900 rounded-xl p-4 border border-gray-700">
            <div className="text-sm text-gray-400 mb-2">Roll</div>
            <div className="grid grid-cols-2 gap-2">
              <div>
                <div className="text-xs text-gray-500">RMSE</div>
                <div className="text-xl font-bold text-white">
                  {error_metrics?.rmse_roll?.toFixed(3)}°
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">MAE</div>
                <div className="text-xl font-bold text-white">
                  {error_metrics?.mae_roll?.toFixed(3)}°
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Varianza</div>
                <div className="text-lg font-mono text-gray-300">
                  {error_metrics?.variance_roll?.toExponential(3)}°²
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Desv. Estándar</div>
                <div className="text-lg font-mono text-gray-300">
                  {error_metrics?.std_dev_roll?.toFixed(4)}°
                </div>
              </div>
            </div>
          </div>

          <div className="bg-gray-900 rounded-xl p-4 border border-gray-700">
            <div className="text-sm text-gray-400 mb-2">Pitch</div>
            <div className="grid grid-cols-2 gap-2">
              <div>
                <div className="text-xs text-gray-500">RMSE</div>
                <div className="text-xl font-bold text-white">
                  {error_metrics?.rmse_pitch?.toFixed(3)}°
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">MAE</div>
                <div className="text-xl font-bold text-white">
                  {error_metrics?.mae_pitch?.toFixed(3)}°
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Varianza</div>
                <div className="text-lg font-mono text-gray-300">
                  {error_metrics?.variance_pitch?.toExponential(3)}°²
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Desv. Estándar</div>
                <div className="text-lg font-mono text-gray-300">
                  {error_metrics?.std_dev_pitch?.toFixed(4)}°
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Comparación Raw vs Kalman */}
      <div>
        <h3 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
          <span className="w-2 h-2 bg-green-500 rounded-full"></span>
          Filtro Kalman
        </h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div className="bg-gray-900 rounded-xl p-4 border border-gray-700">
            <div className="text-sm text-gray-400 mb-2">Roll</div>
            <div className="grid grid-cols-3 gap-2 text-center">
              <div>
                <div className="text-xs text-gray-500">Raw</div>
                <div className="text-lg font-bold text-orange-400">
                  {comparison_roll?.raw_rms?.toFixed(3)}°
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Kalman</div>
                <div className="text-lg font-bold text-green-400">
                  {comparison_roll?.kalman_rms?.toFixed(3)}°
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Mejora</div>
                <div className="text-lg font-bold text-blue-400">
                  {comparison_roll?.improvement_percent?.toFixed(1)}%
                </div>
              </div>
            </div>
          </div>

          <div className="bg-gray-900 rounded-xl p-4 border border-gray-700">
            <div className="text-sm text-gray-400 mb-2">Pitch</div>
            <div className="grid grid-cols-3 gap-2 text-center">
              <div>
                <div className="text-xs text-gray-500">Raw</div>
                <div className="text-lg font-bold text-orange-400">
                  {comparison_pitch?.raw_rms?.toFixed(3)}°
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Kalman</div>
                <div className="text-lg font-bold text-green-400">
                  {comparison_pitch?.kalman_rms?.toFixed(3)}°
                </div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Mejora</div>
                <div className="text-lg font-bold text-blue-400">
                  {comparison_pitch?.improvement_percent?.toFixed(1)}%
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Resumen */}
      <div className="bg-gradient-to-r from-green-900/30 to-blue-900/30 rounded-xl p-4 border border-green-500/30">
        <div className="flex justify-between items-center">
          <span className="text-gray-300">📊 Duración del vuelo</span>
          <span className="font-mono text-white">
            {data.duration_sec?.toFixed(1)} segundos
          </span>
        </div>
        <div className="flex justify-between items-center mt-2">
          <span className="text-gray-300">📈 Muestras procesadas</span>
          <span className="font-mono text-white">{data.sample_count}</span>
        </div>
      </div>
    </div>
  );
}
