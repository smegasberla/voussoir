import React from "react";
import {
  AbsoluteFill,
  Img,
  interpolate,
  spring,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import { C } from "./theme";

// Shared pieces. Every component is frame-driven so the render is
// deterministic: no clocks, no randomness, no CSS animations.

export const Full: React.FC<React.PropsWithChildren<{ style?: React.CSSProperties }>> = ({
  style,
  children,
}) => (
  <AbsoluteFill
    style={{
      backgroundColor: C.slate900,
      fontFamily: "Inter",
      justifyContent: "center",
      alignItems: "center",
      ...style,
    }}
  >
    {children}
  </AbsoluteFill>
);

// The grain the site wears: faint vertical joints, like a cut face of stone.
export const Grain: React.FC = () => (
  <AbsoluteFill
    style={{
      backgroundImage:
        "repeating-linear-gradient(90deg, rgba(255,255,255,0.014) 0 1px, transparent 1px 3px)",
      pointerEvents: "none",
    }}
  />
);

// Words that rise and settle into place, one after another, Apple-style.
export const RiseWords: React.FC<{
  text: string;
  startFrame?: number;
  stagger?: number;
  size?: number;
  weight?: number;
  color?: string;
  accentWords?: string[];
  accentColor?: string;
  tracking?: string;
  lineHeight?: number;
  align?: string;
}> = ({
  text,
  startFrame = 0,
  stagger = 5,
  size = 96,
  weight = 800,
  color = C.chalk,
  accentWords = [],
  accentColor = C.ember,
  tracking = "-0.03em",
  lineHeight = 1.12,
  align = "center",
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const words = text.split(" ");
  return (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        justifyContent: align === "center" ? "center" : "flex-start",
        gap: `0 ${size * 0.28}px`,
        fontSize: size,
        fontWeight: weight,
        lineHeight,
        letterSpacing: tracking,
        color,
      }}
    >
      {words.map((word, i) => {
        const t = spring({
          frame: frame - startFrame - i * stagger,
          fps,
          config: { damping: 16, stiffness: 130, mass: 0.9 },
        });
        const accent = accentWords.includes(word.replace(/[^a-zA-Z]/g, ""));
        return (
          <span
            key={i}
            style={{
              display: "inline-block",
              transform: `translateY(${(1 - t) * 44}px) scale(${0.92 + 0.08 * t})`,
              opacity: t,
              color: accent ? accentColor : undefined,
              textShadow: accent ? `0 0 ${size * 0.5}px rgba(249,138,40,0.35)` : undefined,
            }}
          >
            {word}
          </span>
        );
      })}
    </div>
  );
};

// One line of code that types itself. Shows `chars` characters by this frame.
export const Typed: React.FC<{
  text: string;
  startFrame: number;
  cps?: number;
  size?: number;
  color?: string;
  mono?: boolean;
}> = ({ text, startFrame, cps = 42, size = 30, color = C.chalk, mono = true }) => {
  const frame = useCurrentFrame();
  const chars = Math.max(0, Math.min(text.length, Math.floor((frame - startFrame) * (cps / 30))));
  const caret = frame - startFrame >= 0 && frame - startFrame < text.length / (cps / 30) + 20;
  return (
    <div
      style={{
        fontFamily: mono ? "JetBrains Mono, Menlo, monospace" : "Inter",
        fontSize: size,
        color,
        whiteSpace: "pre",
        display: "flex",
        alignItems: "center",
      }}
    >
      <span>{text.slice(0, chars)}</span>
      <span
        style={{
          width: size * 0.55,
          height: size * 1.05,
          marginLeft: 2,
          background: C.ember,
          opacity: caret ? (Math.floor(frame / 8) % 2 === 0 ? 1 : 0) : 0,
        }}
      />
    </div>
  );
};

// A floating fade used by nearly every scene transition.
export const useFade = (inFrames = 18, outFrames = 18, total: number, delay = 0) => {
  const frame = useCurrentFrame();
  const fadeIn = interpolate(frame - delay, [0, inFrames], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const fadeOut = interpolate(frame, [total - outFrames, total], [1, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  return Math.min(fadeIn, fadeOut);
};

// AVoussoir window mock: dark chrome, mortar hairlines, sidebar and content.
export const WindowMock: React.FC<{
  width: number;
  height: number;
  scale?: number;
  narrow?: boolean;
  children?: React.ReactNode;
  opacity?: number;
}> = ({ width, height, scale = 1, narrow = false, children, opacity = 1 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const pop = spring({ frame, fps, config: { damping: 14, stiffness: 110 } });
  return (
    <div
      style={{
        width: width * scale,
        height: height * scale,
        opacity: opacity * pop,
        transform: `scale(${scale}) translateY(${(1 - pop) * 26}px)`,
        borderRadius: 14,
        overflow: "hidden",
        background: C.slate850,
        border: `1px solid ${C.stone600}`,
        boxShadow: "0 40px 90px rgba(0,0,0,0.55)",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <div
        style={{
          height: 34,
          background: C.slate800,
          borderBottom: `1px solid ${C.stone700}`,
          display: "flex",
          alignItems: "center",
          paddingLeft: 12,
          gap: 8,
          flexShrink: 0,
        }}
      >
        {[C.emberDeep, C.stone500, C.stone500].map((c, i) => (
          <div key={i} style={{ width: 10, height: 10, borderRadius: 5, background: c }} />
        ))}
        <div
          style={{
            marginLeft: 12,
            fontFamily: "JetBrains Mono, Menlo, monospace",
            fontSize: 12,
            color: C.dustDim,
          }}
        >
          {narrow ? "main window" : "voussoir"}
        </div>
      </div>
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>{children}</div>
    </div>
  );
};

export const Sidebar: React.FC<{ width: number; items?: string[] }> = ({
  width,
  items = ["Overview", "Storage", "Settings", "Diagnostics"],
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <div
      style={{
        width,
        background: C.slate900,
        borderRight: `1px solid ${C.stone700}`,
        padding: "14px 0",
        flexShrink: 0,
      }}
    >
      {items.map((it, i) => {
        const on = spring({ frame: frame - 12 - i * 5, fps });
        return (
          <div
            key={it}
            style={{
              padding: "8px 16px",
              fontSize: 13,
              color: i === 0 ? C.emberBright : C.dust,
              borderLeft: i === 0 ? `2px solid ${C.ember}` : "2px solid transparent",
              opacity: on,
              transform: `translateX(${(1 - on) * -10}px)`,
            }}
          >
            {it}
          </div>
        );
      })}
    </div>
  );
};

export const ContentLines: React.FC<{ pad?: number }> = ({ pad = 20 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <div style={{ flex: 1, padding: pad }}>
      {[0.9, 0.62, 0.78, 0.44].map((w, i) => {
        const on = spring({ frame: frame - 18 - i * 6, fps });
        return (
          <div
            key={i}
            style={{
              height: 12,
              width: `${w * 100}%`,
              borderRadius: 6,
              marginBottom: 16,
              background: i === 0 ? C.stone600 : C.stone700,
              opacity: on,
              transform: `translateY(${(1 - on) * 12}px)`,
            }}
          />
        );
      })}
    </div>
  );
};

// The brand mark: the arch SVG from the site, used as the keystone.
export const Mark: React.FC<{ size?: number; opacity?: number }> = ({ size = 220, opacity = 1 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const pop = spring({ frame, fps, config: { damping: 13, stiffness: 90 } });
  return (
    <div
      style={{
        width: size,
        height: size,
        opacity: opacity * pop,
        transform: `scale(${0.7 + 0.3 * pop})`,
        filter: `drop-shadow(0 0 ${size * 0.18}px rgba(249,138,40,0.45))`,
      }}
    >
      <Img src={staticFile("voussoir.svg")} style={{ width: "100%", height: "100%" }} />
    </div>
  );
};
