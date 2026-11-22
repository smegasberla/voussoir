import React from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { C } from "./theme";
import {
  ContentLines,
  Full,
  Grain,
  Mark,
  RiseWords,
  Sidebar,
  Typed,
  WindowMock,
  useFade,
} from "./components";

// Scene lengths in frames (30 fps). Root.tsx composes them back to back.
export const LEN = {
  intro: 150,
  writeRust: 180,
  artifact: 180,
  commands: 210,
  allowlists: 180,
  hotReload: 180,
  mobile: 180,
  outro: 180,
};

// ------------------------------------------------------------- 1. intro

export const Intro: React.FC = () => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const total = LEN.intro;
  const fade = useFade(14, 20, total);
  const tag = spring({ frame: f - 55, fps, config: { damping: 15 } });
  return (
    <Full style={{ opacity: fade }}>
      <Grain />
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 42 }}>
        <Mark size={230} />
        <div
          style={{
            fontSize: 44,
            fontWeight: 500,
            color: C.dust,
            letterSpacing: "0.14em",
            opacity: tag,
            transform: `translateY(${(1 - tag) * 18}px)`,
          }}
        >
          A DESKTOP AND MOBILE APP FRAMEWORK
        </div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 2. write rust

export const WriteRust: React.FC = () => {
  const total = LEN.writeRust;
  const fade = useFade(12, 20, total);
  return (
    <Full style={{ opacity: fade }}>
      <Grain />
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 56 }}>
        <RiseWords
          text="You write Rust."
          startFrame={12}
          size={130}
          accentWords={["Rust"]}
        />
        <RiseWords
          text="Your users just click."
          startFrame={70}
          size={130}
          color={C.dust}
          accentWords={["click"]}
        />
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 3. artifact

export const Artifact: React.FC = () => {
  const total = LEN.artifact;
  const fade = useFade(12, 20, total);
  const frame = useCurrentFrame();
  const caption = interpolate(frame, [95, 115], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  return (
    <Full style={{ opacity: fade, gap: 60, flexDirection: "column" }}>
      <Grain />
      <RiseWords
        text="What ships is a Rust binary."
        startFrame={8}
        size={92}
        accentWords={["binary"]}
      />
      <WindowMock width={880} height={430} scale={0.95}>
        <Sidebar width={190} />
        <ContentLines />
      </WindowMock>
      <div
        style={{
          fontSize: 30,
          fontWeight: 600,
          color: C.dust,
          opacity: caption,
          letterSpacing: "0.06em",
        }}
      >
        ONE BINARY. EVERY PLATFORM.
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 4. commands

const CodeCard: React.FC<{
  children: React.ReactNode;
  x: number;
  opacity: number;
  width: number;
}> = ({ children, x, opacity, width }) => (
  <div
    style={{
      transform: `translateX(${x}px)`,
      opacity,
      width,
      background: C.slate850,
      border: `1px solid ${C.stone700}`,
      borderRadius: 12,
      padding: "22px 26px",
      boxShadow: "0 24px 60px rgba(0,0,0,0.45)",
    }}
  >
    {children}
  </div>
);

export const Commands: React.FC = () => {
  const total = LEN.commands;
  const fade = useFade(12, 20, total);
  const frame = useCurrentFrame();
  const okIn = interpolate(frame, [95, 115], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const errIn = interpolate(frame, [140, 160], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  return (
    <Full style={{ opacity: fade, gap: 40, flexDirection: "column" }}>
      <Grain />
      <RiseWords
        text="Typed commands, in and out."
        startFrame={6}
        size={84}
        accentWords={["typed"]}
      />
      <div style={{ display: "flex", gap: 34, alignItems: "center" }}>
        <CodeCard x={0} opacity={1} width={760}>
          <div style={{ color: C.dustDim, fontSize: 15, marginBottom: 14 }}>
            frontend / main.ts
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <Typed
              text={`const greeting = await voussoir.invoke<string>(`}
              startFrame={28}
              size={26}
            />
            <Typed text={`  "greet", { name: "you" });`} startFrame={62} size={26} />
          </div>
        </CodeCard>
        <div
          style={{
            opacity: okIn,
            transform: `scale(${0.9 + 0.1 * okIn})`,
            padding: "18px 30px",
            background: "rgba(249,138,40,0.12)",
            border: `1px solid ${C.ember}`,
            borderRadius: 12,
            fontFamily: "JetBrains Mono, Menlo, monospace",
            fontSize: 26,
            color: C.emberBright,
          }}
        >
          "Hello, you"
        </div>
      </div>
      <div style={{ display: "flex", gap: 34, alignItems: "center" }}>
        <CodeCard x={0} opacity={errIn} width={560}>
          <Typed
            text={`if (err.kind === "payload") {`}
            startFrame={146}
            size={26}
            color={C.emberBright}
          />
          <Typed text={`  showHint(err)`} startFrame={172} size={26} color={C.dust} />
        </CodeCard>
        <div
          style={{
            opacity: errIn,
            fontSize: 30,
            fontWeight: 600,
            color: C.dust,
            letterSpacing: "0.04em",
          }}
        >
          errors the page can branch on
        </div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 5. allowlists

const Chip: React.FC<{ label: string; on: boolean; ember?: boolean }> = ({
  label,
  on,
  ember = false,
}) => (
  <div
    style={{
      fontFamily: "JetBrains Mono, Menlo, monospace",
      fontSize: 17,
      padding: "9px 16px",
      borderRadius: 8,
      border: `1px solid ${ember ? C.ember : C.stone500}`,
      color: ember ? C.emberBright : C.dust,
      background: ember ? "rgba(249,138,40,0.10)" : C.slate800,
      opacity: on ? 1 : 0.15,
    }}
  >
    {label}
  </div>
);

export const Allowlists: React.FC = () => {
  const total = LEN.allowlists;
  const fade = useFade(12, 20, total);
  const frame = useCurrentFrame();
  const all = ["storage.get", "storage.set", "shell.exec", "window.open", "dialog.open"];
  const few = ["storage.get"];
  return (
    <Full style={{ opacity: fade, gap: 46, flexDirection: "column" }}>
      <Grain />
      <RiseWords
        text="Every window gets its own command surface."
        startFrame={6}
        size={76}
        accentWords={["own"]}
      />
      <div style={{ display: "flex", gap: 60, alignItems: "flex-start" }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 18, alignItems: "center" }}>
          <WindowMock width={430} height={300} scale={1}>
            <Sidebar width={110} items={["Home"]} />
            <ContentLines pad={16} />
          </WindowMock>
          <div style={{ display: "flex", gap: 10, flexWrap: "wrap", justifyContent: "center", maxWidth: 460 }}>
            {all.map((c, i) => (
              <Chip key={c} label={c} on={frame > 30 + i * 8} />
            ))}
          </div>
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 18, alignItems: "center" }}>
          <WindowMock width={430} height={300} scale={1} narrow>
            <Sidebar width={110} items={["Guest"]} />
            <ContentLines pad={16} />
          </WindowMock>
          <div style={{ display: "flex", gap: 10 }}>
            {few.map((c) => (
              <Chip key={c} label={c} on={frame > 70} ember />
            ))}
          </div>
        </div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 6. hot reload

export const HotReload: React.FC = () => {
  const total = LEN.hotReload;
  const fade = useFade(12, 20, total);
  const frame = useCurrentFrame();
  const flash = interpolate(frame, [110, 118, 126], [0, 0.5, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const swapped = frame >= 118;
  const zero = spring({
    frame: frame - 130,
    fps: 30,
    config: { damping: 12, stiffness: 120 },
  });
  return (
    <Full style={{ opacity: fade, gap: 44, flexDirection: "column" }}>
      <Grain />
      <RiseWords text="Edit. Save. There it is." startFrame={6} size={92} accentWords={["there"]} />
      <CodeCard x={0} opacity={1} width={780}>
        <div style={{ color: C.dustDim, fontSize: 15, marginBottom: 14 }}>commands / greet.rs</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <Typed text={`fn greet(name: &str) -> String {`} startFrame={28} size={26} />
          <Typed
            text={swapped ? `    format!("Hello again, {name}!")` : `    format!("Hello, {name}!")`}
            startFrame={46}
            size={26}
            color={swapped ? C.emberBright : C.chalk}
          />
          <Typed text={`}`} startFrame={64} size={26} />
        </div>
        <AbsoluteFill
          style={{
            background: C.ember,
            opacity: flash,
            borderRadius: 12,
            mixBlendMode: "screen",
          }}
        />
      </CodeCard>
      <div
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: 20,
          opacity: zero,
          transform: `translateY(${(1 - zero) * 20}px)`,
        }}
      >
        <div
          style={{
            fontSize: 84,
            fontWeight: 800,
            color: C.ember,
            letterSpacing: "-0.03em",
          }}
        >
          0
        </div>
        <div style={{ fontSize: 32, fontWeight: 600, color: C.dust }}>restarts needed</div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 7. mobile

export const Mobile: React.FC = () => {
  const total = LEN.mobile;
  const fade = useFade(12, 20, total);
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const rise = spring({ frame, fps, config: { damping: 15, stiffness: 90 } });
  return (
    <Full style={{ opacity: fade, gap: 50, flexDirection: "row" }}>
      <Grain />
      <div
        style={{
          transform: `translateY(${(1 - rise) * 90}px)`,
          opacity: rise,
          width: 320,
          height: 640,
          borderRadius: 40,
          border: `3px solid ${C.stone500}`,
          background: C.slate850,
          overflow: "hidden",
          boxShadow: "0 50px 100px rgba(0,0,0,0.6)",
        }}
      >
        <div
          style={{
            height: 46,
            background: C.slate800,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            borderBottom: `1px solid ${C.stone700}`,
          }}
        >
          <div style={{ width: 90, height: 10, borderRadius: 5, background: C.stone600 }} />
        </div>
        <ContentLines pad={24} />
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 34, alignItems: "flex-start" }}>
        <RiseWords
          text="Android."
          startFrame={22}
          size={104}
          align="flex-start"
          accentWords={["Android"]}
        />
        <RiseWords
          text="iOS."
          startFrame={48}
          size={104}
          align="flex-start"
          accentWords={["iOS"]}
        />
        <RiseWords
          text="Same Rust underneath."
          startFrame={76}
          size={52}
          align="flex-start"
          color={C.dust}
          accentWords={["Rust"]}
        />
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 8. outro

export const Outro: React.FC = () => {
  const total = LEN.outro;
  const fade = useFade(14, 26, total);
  const frame = useCurrentFrame();
  const cmd = "curl -fsSL https://voussoir.dev/install.sh | sh";
  const gh = interpolate(frame, [130, 150], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  return (
    <Full style={{ opacity: fade, gap: 40, flexDirection: "column" }}>
      <Grain />
      <Mark size={170} />
      <RiseWords text="Build with the keystone." startFrame={30} size={72} accentWords={["keystone"]} />
      <div
        style={{
          background: C.slate850,
          border: `1px solid ${C.stone700}`,
          borderRadius: 12,
          padding: "20px 30px",
          boxShadow: "0 24px 60px rgba(0,0,0,0.45)",
        }}
      >
        <Typed text={cmd} startFrame={58} cps={38} size={28} color={C.emberBright} />
      </div>
      <div
        style={{
          fontSize: 26,
          color: C.dust,
          opacity: gh,
          fontFamily: "JetBrains Mono, Menlo, monospace",
        }}
      >
        github.com/smegasberla/voussoir
      </div>
    </Full>
  );
};
