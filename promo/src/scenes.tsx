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

// Scene lengths in frames (30 fps). Root.tsx composes them back to back; the
// audio cue times in make-audio.ts derive from these, keep both in sync.
export const LEN = {
  intro: 150,
  writeRust: 180,
  artifact: 180,
  structure: 210,
  commands: 210,
  allowlists: 180,
  hotReload: 180,
  mobile: 180,
  outro: 180,
};

// A small pill used for platforms, labels and the bloat that is not there.
const Pill: React.FC<{
  label: string;
  on: number;
  ember?: boolean;
  strike?: boolean;
  size?: number;
}> = ({ label, on, ember = false, strike = false, size = 15 }) => (
  <div
    style={{
      fontFamily: "JetBrains Mono, Menlo, monospace",
      fontSize: size,
      letterSpacing: "0.10em",
      padding: "8px 16px",
      borderRadius: 999,
      border: `1px solid ${ember ? C.ember : C.stone500}`,
      color: strike ? C.dustDim : ember ? C.emberBright : C.chalk,
      background: ember ? "rgba(249,138,40,0.10)" : C.slate800,
      opacity: on,
      transform: `translateY(${(1 - on) * 14}px) scale(${0.9 + 0.1 * on})`,
      textDecoration: strike ? "line-through" : undefined,
      textDecorationColor: C.emberDeep,
      textDecorationThickness: 2,
      whiteSpace: "pre",
    }}
  >
    {label}
  </div>
);

// ------------------------------------------------------------- 1. intro

export const Intro: React.FC = () => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const total = LEN.intro;
  const fade = useFade(14, 20, total);
  const roles = ["DESKTOP", "MOBILE", "RUST"];
  return (
    <Full style={{ opacity: fade }}>
      <Grain />
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 34 }}>
        <Mark size={230} />
        <RiseWords text="voussoir" startFrame={12} size={88} tracking="-0.04em" />
        <div style={{ display: "flex", gap: 14, marginTop: 6 }}>
          {roles.map((r, i) => {
            const on = spring({ frame: f - 40 - i * 15, fps, config: { damping: 13 } });
            return <Pill key={r} label={r} on={on} ember={i === 2} />;
          })}
        </div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 2. write rust

export const WriteRust: React.FC = () => {
  const total = LEN.writeRust;
  const fade = useFade(12, 20, total);
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  // The line says click, so something on screen actually gets clicked: a
  // button that presses under an expanding ring.
  const press = 1 - 0.08 * interpolate(f, [108, 116, 124], [0, 1, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const ring = interpolate(f, [116, 146], [0.6, 2.4], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const ringFade = interpolate(f, [116, 124, 146], [0, 0.55, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  return (
    <Full style={{ opacity: fade }}>
      <Grain />
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 70 }}>
        <RiseWords text="You write Rust." startFrame={12} size={130} accentWords={["Rust"]} />
        <RiseWords
          text="Your users just click."
          startFrame={70}
          size={130}
          color={C.dust}
          accentWords={["click"]}
        />
        <div style={{ position: "relative", marginTop: 20 }}>
          <div
            style={{
              position: "absolute",
              left: "50%",
              top: "50%",
              width: 150,
              height: 150,
              marginLeft: -75,
              marginTop: -75,
              borderRadius: 999,
              border: `2px solid ${C.ember}`,
              transform: `scale(${ring})`,
              opacity: ringFade,
            }}
          />
          <div
            style={{
              transform: `scale(${press})`,
              padding: "22px 54px",
              borderRadius: 14,
              background: C.ember,
              color: C.mortar,
              fontSize: 26,
              fontWeight: 800,
              letterSpacing: "0.02em",
              boxShadow: "0 18px 40px rgba(249,138,40,0.35)",
              opacity: spring({ frame: f - 92, fps }),
            }}
          >
            Run
          </div>
        </div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 3. artifact

export const Artifact: React.FC = () => {
  const total = LEN.artifact;
  const fade = useFade(12, 20, total);
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const platforms = ["LINUX", "MACOS", "WINDOWS", "ANDROID", "IOS"];
  return (
    <Full style={{ opacity: fade, gap: 54, flexDirection: "column" }}>
      <Grain />
      <RiseWords text="One binary." startFrame={8} size={104} accentWords={["binary"]} />
      <WindowMock width={880} height={400} scale={0.95}>
        <Sidebar width={190} />
        <ContentLines />
      </WindowMock>
      <div style={{ display: "flex", gap: 12 }}>
        {platforms.map((p, i) => {
          const on = spring({ frame: f - 100 - i * 9, fps, config: { damping: 12 } });
          return <Pill key={p} label={p} on={on} />;
        })}
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 4. structure

const TREE: { text: string; color: string; depth: number }[] = [
  { text: "myapp/", color: C.chalk, depth: 0 },
  { text: "index.html", color: C.emberBright, depth: 1 },
  { text: "src/main.ts", color: C.emberBright, depth: 1 },
  { text: "native/main.rs", color: C.chalk, depth: 1 },
  { text: "native/commands/lib.rs", color: C.ember, depth: 1 },
];

export const Structure: React.FC = () => {
  const total = LEN.structure;
  const fade = useFade(12, 20, total);
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const stats: [string, string, number][] = [
    ["LOOK", "ms", 92],
    ["LOGIC", "s", 118],
    ["SHELL", "once", 144],
  ];
  return (
    <Full style={{ opacity: fade, gap: 40, flexDirection: "column" }}>
      <Grain />
      <RiseWords text="One directory." startFrame={6} size={96} accentWords={["directory"]} />
      <div style={{ display: "flex", gap: 70, alignItems: "center" }}>
        <div
          style={{
            background: C.slate850,
            border: `1px solid ${C.stone700}`,
            borderRadius: 12,
            padding: "26px 34px",
            boxShadow: "0 24px 60px rgba(0,0,0,0.45)",
            display: "flex",
            flexDirection: "column",
            gap: 10,
          }}
        >
          {TREE.map((row, i) => {
            const on = spring({ frame: frame - 38 - i * 9, fps });
            return (
              <div
                key={row.text}
                style={{
                  fontFamily: "JetBrains Mono, Menlo, monospace",
                  fontSize: 25,
                  color: row.color,
                  whiteSpace: "pre",
                  opacity: on,
                  transform: `translateX(${(1 - on) * -24}px)`,
                  paddingLeft: row.depth * 34,
                }}
              >
                {row.text}
              </div>
            );
          })}
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 22 }}>
          {stats.map(([who, cost, at]) => {
            const on = spring({ frame: frame - at, fps });
            return (
              <div
                key={who}
                style={{
                  opacity: on,
                  transform: `translateX(${(1 - on) * 30}px)`,
                  display: "flex",
                  alignItems: "baseline",
                  gap: 16,
                }}
              >
                <div
                  style={{
                    fontSize: 58,
                    fontWeight: 800,
                    color: C.ember,
                    letterSpacing: "-0.02em",
                    width: 130,
                  }}
                >
                  {cost}
                </div>
                <div
                  style={{
                    fontSize: 15,
                    fontWeight: 600,
                    color: C.dustDim,
                    letterSpacing: "0.14em",
                  }}
                >
                  {who}
                </div>
              </div>
            );
          })}
        </div>
      </div>
      <div style={{ display: "flex", gap: 14 }}>
        {["node_modules", "bundled runtime"].map((label, i) => {
          const on = spring({ frame: frame - 168 - i * 10, fps, config: { damping: 12 } });
          return <Pill key={label} label={label} on={on} strike />;
        })}
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 5. commands

const CodeCard: React.FC<{
  children: React.ReactNode;
  opacity: number;
  width: number;
}> = ({ children, opacity, width }) => (
  <div
    style={{
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
  const { fps } = useVideoConfig();
  const okIn = spring({ frame: frame - 95, fps, config: { damping: 12 } });
  const errIn = spring({ frame: frame - 140, fps, config: { damping: 12 } });
  return (
    <Full style={{ opacity: fade, gap: 44, flexDirection: "column" }}>
      <Grain />
      <RiseWords text="In. Out." startFrame={6} size={118} accentWords={["Out"]} />
      <div style={{ display: "flex", gap: 34, alignItems: "center" }}>
        <CodeCard opacity={1} width={760}>
          <div style={{ color: C.dustDim, fontSize: 15, marginBottom: 14 }}>frontend / main.ts</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <Typed text={`const greeting = await voussoir.invoke<string>(`} startFrame={28} size={26} />
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
        <CodeCard opacity={errIn} width={560}>
          <Typed text={`if (err.kind === "payload") {`} startFrame={146} size={26} color={C.emberBright} />
          <Typed text={`  showHint(err)`} startFrame={172} size={26} color={C.dust} />
        </CodeCard>
        <div
          style={{
            opacity: errIn,
            transform: `translateX(${(1 - errIn) * -20}px)`,
            padding: "14px 24px",
            border: `1px solid ${C.stone500}`,
            borderRadius: 10,
            fontFamily: "JetBrains Mono, Menlo, monospace",
            fontSize: 22,
            color: C.dust,
          }}
        >
          {'{ kind: "payload" }'}
        </div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 6. allowlists

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
  return (
    <Full style={{ opacity: fade, gap: 46, flexDirection: "column" }}>
      <Grain />
      <RiseWords text="One surface per window." startFrame={6} size={76} accentWords={["surface"]} />
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
            <Chip label="storage.get" on={frame > 70} ember />
          </div>
        </div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 7. hot reload

export const HotReload: React.FC = () => {
  const total = LEN.hotReload;
  const fade = useFade(12, 20, total);
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const flash = interpolate(frame, [110, 118, 126], [0, 0.5, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const swapped = frame >= 118;
  const zero = spring({ frame: frame - 130, fps, config: { damping: 12, stiffness: 120 } });
  const keyPress = interpolate(frame, [104, 110, 118], [1, 0.88, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  return (
    <Full style={{ opacity: fade, gap: 42, flexDirection: "column" }}>
      <Grain />
      <div style={{ display: "flex", alignItems: "center", gap: 26 }}>
        <RiseWords text="Save." startFrame={6} size={130} />
        <div
          style={{
            fontFamily: "JetBrains Mono, Menlo, monospace",
            fontSize: 26,
            padding: "6px 16px",
            borderRadius: 8,
            border: `1px solid ${C.stone500}`,
            color: C.dust,
            background: C.slate800,
            transform: `scale(${keyPress})`,
            opacity: spring({ frame: frame - 90, fps }),
          }}
        >
          Ctrl+S
        </div>
      </div>
      <CodeCard opacity={1} width={780}>
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
        <div style={{ fontSize: 84, fontWeight: 800, color: C.ember, letterSpacing: "-0.03em" }}>0</div>
        <div style={{ fontSize: 32, fontWeight: 600, color: C.dust }}>restarts</div>
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 8. mobile

export const Mobile: React.FC = () => {
  const total = LEN.mobile;
  const fade = useFade(12, 20, total);
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const rise = spring({ frame, fps, config: { damping: 15, stiffness: 90 } });
  return (
    <Full style={{ opacity: fade, gap: 80, flexDirection: "row" }}>
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
      <div style={{ display: "flex", flexDirection: "column", gap: 26 }}>
        {[
          ["Android", 22],
          ["iOS", 48],
        ].map(([label, at]) => {
          const on = spring({ frame: frame - (at as number), fps, config: { damping: 12 } });
          return (
            <div
              key={label}
              style={{
                opacity: on,
                transform: `translateX(${(1 - on) * 40}px)`,
                padding: "20px 40px",
                borderRadius: 16,
                border: `1px solid ${C.stone600}`,
                background: C.slate850,
                fontSize: 54,
                fontWeight: 800,
                color: C.chalk,
                letterSpacing: "-0.02em",
              }}
            >
              {label}
            </div>
          );
        })}
      </div>
    </Full>
  );
};

// ------------------------------------------------------------- 9. outro

export const Outro: React.FC = () => {
  const total = LEN.outro;
  const fade = useFade(14, 26, total);
  const frame = useCurrentFrame();
  const unix = "curl -fsSL raw.githubusercontent.com/smegasberla/voussoir/master/install.sh | sh";
  const win = "irm raw.githubusercontent.com/smegasberla/voussoir/master/install.ps1 | iex";
  const gh = interpolate(frame, [126, 146], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const ai = interpolate(frame, [140, 156], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  return (
    <Full style={{ opacity: fade, gap: 34, flexDirection: "column" }}>
      <Grain />
      <Mark size={140} />
      <RiseWords text="Build with the keystone." startFrame={24} size={58} accentWords={["keystone"]} />
      <div
        style={{
          background: C.slate850,
          border: `1px solid ${C.stone700}`,
          borderRadius: 12,
          padding: "24px 34px",
          boxShadow: "0 24px 60px rgba(0,0,0,0.45)",
          display: "flex",
          flexDirection: "column",
          gap: 22,
          minWidth: 0,
        }}
      >
        {[
          ["LINUX · MACOS · BSD", unix, 48],
          ["WINDOWS", win, 98],
        ].map(([label, cmd, start]) => (
          <div key={label as string} style={{ minWidth: 0 }}>
            <div
              style={{
                fontSize: 14,
                fontWeight: 600,
                color: C.dustDim,
                letterSpacing: "0.14em",
                marginBottom: 8,
                whiteSpace: "pre",
              }}
            >
              {label}
            </div>
            <Typed text={cmd as string} startFrame={start as number} cps={50} size={21} color={C.emberBright} />
          </div>
        ))}
      </div>
      <div
        style={{
          fontSize: 24,
          color: C.dust,
          opacity: gh,
          fontFamily: "JetBrains Mono, Menlo, monospace",
        }}
      >
        github.com/smegasberla/voussoir
      </div>
      <div
        style={{
          fontSize: 17,
          color: C.dustDim,
          opacity: ai,
          letterSpacing: "0.04em",
        }}
      >
        MIT licensed. Written with AI assistance, and that is disclosed rather than implied.
      </div>
    </Full>
  );
};
