"use client";

import { motion } from "framer-motion";

interface UncertaintyChartProps {
  data: any;
}

export default function UncertaintyChart({ data }: UncertaintyChartProps) {
  if (!data?.budget?.sources) return null;

  const { sources, standard_uncertainty, expanded_uncertainty_k2 } =
    data.budget;

  // Colores para cada fuente
  const colors = [
    "bg-blue-500",
    "bg-red-500",
    "bg-green-500",
    "bg-yellow-500",
    "bg-purple-500",
  ];

  // Encontrar el valor máximo para escalar las barras
  const maxValue = Math.max(
    ...sources.map((s: any) => s.value),
    expanded_uncertainty_k2,
  );

  return (
    <div className="space-y-6">
      <h3 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
        <span className="w-2 h-2 bg-purple-500 rounded-full"></span>
        Presupuesto de Incertidumbre (GUM)
      </h3>

      {/* Barras de contribución */}
      <div className="space-y-3">
        {sources.map((source: any, idx: number) => {
          const percentage = (source.value / maxValue) * 100;
          return (
            <motion.div
              key={source.name}
              initial={{ opacity: 0, x: -20 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ delay: idx * 0.1 }}
            >
              <div className="flex justify-between text-sm mb-1">
                <span className="text-gray-300">{source.name}</span>
                <span className="text-gray-400 font-mono">
                  {source.value.toFixed(4)}°
                </span>
              </div>
              <div className="h-8 bg-gray-700 rounded-full overflow-hidden">
                <motion.div
                  initial={{ width: 0 }}
                  animate={{ width: `${percentage}%` }}
                  transition={{ duration: 0.5, delay: idx * 0.1 }}
                  className={`h-full ${colors[idx % colors.length]} rounded-full flex items-center justify-end px-2 text-xs font-bold text-white`}
                >
                  {percentage > 15 ? source.value.toFixed(3) : ""}
                </motion.div>
              </div>
            </motion.div>
          );
        })}
      </div>

      {/* Línea divisoria */}
      <div className="border-t border-gray-700 my-4"></div>

      {/* Resultados combinados */}
      <div className="grid grid-cols-2 gap-4">
        <div className="bg-gray-900 rounded-xl p-4 text-center">
          <div className="text-sm text-gray-400 mb-1">
            Incertidumbre estándar (k=1)
          </div>
          <div className="text-2xl font-bold text-blue-400">
            {standard_uncertainty?.toFixed(4)}°
          </div>
        </div>
        <div className="bg-gray-900 rounded-xl p-4 text-center">
          <div className="text-sm text-gray-400 mb-1">
            Incertidumbre expandida (k=2)
          </div>
          <div className="text-2xl font-bold text-green-400">
            {expanded_uncertainty_k2?.toFixed(4)}°
          </div>
          <div className="text-xs text-gray-500 mt-1">95% de confianza</div>
        </div>
      </div>

      {/* Descripción de distribución */}
      <div className="bg-gray-900/50 rounded-xl p-4">
        <h4 className="text-sm font-semibold text-gray-300 mb-2">
          📐 Distribuciones estadísticas
        </h4>
        <div className="space-y-1 text-xs text-gray-400">
          {sources.map((source: any) => {
            let distText = "";
            if (source.distribution.Normal) {
              distText = `Normal (μ=0, σ=${source.distribution.Normal.std_dev.toFixed(4)})`;
            } else if (source.distribution.Uniform) {
              distText = `Uniforme [${source.distribution.Uniform.min.toFixed(4)}, ${source.distribution.Uniform.max.toFixed(4)}]`;
            }
            return (
              <div key={source.name} className="flex justify-between">
                <span>{source.name}</span>
                <span className="font-mono">{distText}</span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
