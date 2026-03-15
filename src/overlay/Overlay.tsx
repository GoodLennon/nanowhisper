import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";

type OverlayState = "recording" | "transcribing";

function Overlay() {
  const [state, setState] = useState<OverlayState>("recording");
  const levelRef = useRef(0);
  const smoothLevelRef = useRef(0);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const phaseRef = useRef(0);

  useEffect(() => {
    const unlisten1 = listen<number>("audio-level", (e) => {
      levelRef.current = e.payload;
    });
    const unlisten2 = listen("transcribing", () => {
      setState("transcribing");
    });
    return () => {
      unlisten1.then((f) => f());
      unlisten2.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || state !== "recording") return;

    const ctx = canvas.getContext("2d")!;
    let animId: number;

    const draw = () => {
      const w = canvas.width;
      const h = canvas.height;
      ctx.clearRect(0, 0, w, h);

      // Smooth the level for natural transitions
      const target = levelRef.current;
      const smoothed = smoothLevelRef.current;
      smoothLevelRef.current = smoothed + (target - smoothed) * 0.15;
      const level = smoothLevelRef.current;

      // Silence threshold
      const isSilent = level < 0.008;

      phaseRef.current += isSilent ? 0.02 : 0.08;

      ctx.beginPath();
      ctx.moveTo(0, h / 2);

      for (let x = 0; x < w; x++) {
        const t = x / w;
        let y: number;

        if (isSilent) {
          // Flat line with subtle breathing pulse
          y = h / 2 + Math.sin(t * Math.PI * 2 + phaseRef.current) * 1.5;
        } else {
          // Wave amplitude proportional to voice level
          const amplitude = level * h * 0.45;
          y =
            h / 2 +
            Math.sin(t * Math.PI * 3 + phaseRef.current) * amplitude +
            Math.sin(t * Math.PI * 5 + phaseRef.current * 1.3) * amplitude * 0.3 +
            Math.sin(t * Math.PI * 7 + phaseRef.current * 0.7) * amplitude * 0.1;
        }

        ctx.lineTo(x, y);
      }

      // Color shifts with level: cyan when quiet, brighter when loud
      const alpha = isSilent ? 0.4 : 0.5 + level * 0.5;
      ctx.strokeStyle = `rgba(56, 189, 248, ${alpha})`;
      ctx.lineWidth = isSilent ? 1.5 : 2 + level * 2;
      ctx.stroke();

      animId = requestAnimationFrame(draw);
    };

    draw();
    return () => cancelAnimationFrame(animId);
  }, [state]);

  return (
    <div className="overlay-container">
      {state === "recording" ? (
        <canvas ref={canvasRef} width={320} height={48} className="wave-canvas" />
      ) : (
        <div className="transcribing-text">Transcribing...</div>
      )}
    </div>
  );
}

export default Overlay;
