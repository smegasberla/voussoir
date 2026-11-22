import React from "react";
import { Audio, Composition, Sequence, interpolate, staticFile } from "remotion";
import "./fonts";
import { FPS } from "./theme";
import * as S from "./scenes";

// Eight scenes, back to back, one continuous take. Total: 1440 frames = 48 s.
const sequence = [
  [S.Intro, S.LEN.intro],
  [S.WriteRust, S.LEN.writeRust],
  [S.Artifact, S.LEN.artifact],
  [S.Commands, S.LEN.commands],
  [S.Allowlists, S.LEN.allowlists],
  [S.HotReload, S.LEN.hotReload],
  [S.Mobile, S.LEN.mobile],
  [S.Outro, S.LEN.outro],
] as const;

const DURATION = sequence.reduce((sum, [, len]) => sum + len, 0);

export const RemotionRoot: React.FC = () => {
  const duration = sequence.reduce((sum, [, len]) => sum + len, 0);
  return (
    <Composition
      id="VoussoirPromo"
      component={Runner as React.FC}
      durationInFrames={duration}
      fps={FPS}
      width={1920}
      height={1080}
      defaultProps={{}}
    />
  );
};

// Render every scene in order; Remotion supplies the frame offset itself, so
// each scene keeps counting from zero inside its own slice.
const Runner: React.FC = () => {
  let cursor = 0;
  return (
    <>
      {sequence.map(([Scene, len], i) => {
        const from = cursor;
        cursor += len;
        const Cmp = Scene as React.FC;
        return (
          <Sequence key={i} from={from} durationInFrames={len}>
            <Cmp />
          </Sequence>
        );
      })}
      <Audio
        src={staticFile("music.wav")}
        volume={(f) =>
          0.9 *
          interpolate(f, [0, 15, DURATION - 30, DURATION], [0, 1, 1, 0], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          })
        }
      />
    </>
  );
};
