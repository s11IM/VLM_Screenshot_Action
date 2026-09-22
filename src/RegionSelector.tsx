import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState, type PointerEvent } from "react";

type Point = { x: number; y: number };

export default function RegionSelector() {
  const [start, setStart] = useState<Point | null>(null);
  const [current, setCurrent] = useState<Point | null>(null);

  useEffect(() => {
    const cancel = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") void invoke("cancel_region_selection");
    };
    window.addEventListener("keydown", cancel);
    return () => window.removeEventListener("keydown", cancel);
  }, []);

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    const point = { x: event.clientX, y: event.clientY };
    setStart(point);
    setCurrent(point);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (start) setCurrent({ x: event.clientX, y: event.clientY });
  };

  const onPointerUp = async (event: PointerEvent<HTMLDivElement>) => {
    if (!start) return;
    const end = { x: event.clientX, y: event.clientY };
    const x = Math.min(start.x, end.x);
    const y = Math.min(start.y, end.y);
    const width = Math.abs(end.x - start.x);
    const height = Math.abs(end.y - start.y);
    if (width < 24 || height < 24) {
      setStart(null);
      setCurrent(null);
      return;
    }
    await invoke("complete_region_selection", {
      x,
      y,
      width,
      height,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
    });
  };

  const rect = start && current
    ? {
        left: Math.min(start.x, current.x),
        top: Math.min(start.y, current.y),
        width: Math.abs(current.x - start.x),
        height: Math.abs(current.y - start.y),
      }
    : null;

  return (
    <div
      className="region-selector"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    >
      <div className="selector-tip">
        <strong>框选目标区域</strong>
        <span>拖动鼠标选择，Esc 取消</span>
      </div>
      {rect && (
        <div className="selection-rect" style={rect}>
          <span>{Math.round(rect.width)} × {Math.round(rect.height)}</span>
        </div>
      )}
    </div>
  );
}
