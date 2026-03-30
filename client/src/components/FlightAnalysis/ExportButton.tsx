"use client";

import { useState } from "react";
import html2canvas from "html2canvas-pro";

interface ExportButtonProps {
  targetRef: React.RefObject<HTMLDivElement | null>;
  filename?: string;
}

export default function ExportButton({
  targetRef,
  filename = "flight_analysis",
}: ExportButtonProps) {
  const [isExporting, setIsExporting] = useState(false);

  const handleExport = async () => {
    if (!targetRef.current) return;

    setIsExporting(true);

    try {
      // Esperar un poco para que los gráficos estén renderizados
      await new Promise((resolve) => setTimeout(resolve, 100));

      const canvas = await html2canvas(targetRef.current, {
        scale: 3, // Alta resolución (3x)
        backgroundColor: "#111827", // Fondo gris oscuro (bg-gray-900)
        logging: false,
        useCORS: true,
        allowTaint: true,  
      });

      // Crear enlace de descarga
      const link = document.createElement("a");
      const timestamp = new Date()
        .toISOString()
        .slice(0, 19)
        .replace(/:/g, "-");
      link.download = `${filename}_${timestamp}.png`;
      link.href = canvas.toDataURL("image/png");
      link.click();
    } catch (error) {
      console.error("Error exporting image:", error);
    } finally {
      setIsExporting(false);
    }
  };

  return (
    <button
      onClick={handleExport}
      disabled={isExporting}
      className="flex items-center gap-2 px-4 py-2 bg-purple-600 hover:bg-purple-700 rounded-lg text-sm font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
    >
      {isExporting ? (
        <>
          <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white"></div>
          Exportando...
        </>
      ) : (
        <>📸 Exportar Reporte</>
      )}
    </button>
  );
}
