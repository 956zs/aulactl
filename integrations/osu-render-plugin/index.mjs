// osu-render plugin (API 1): the keyboard recorded with `aulactl feed --record`, drawn by
// `aulactl replay overlay` on the replay's song clock and laid over danser's video by the host.
import { accessSync, constants } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

/** aulactl's exit code when no recorded session lines up with the replay. */
const NO_SESSION_EXIT = 3;
/** Overlay canvas aspect (width / height). */
const ASPECT = 9 / 16;
const FPS = 60;

/** AULACTL (an absolute path), else aulactl on PATH, else ~/.local/bin/aulactl. */
export function findAulactl(env = process.env) {
  const configured = env.AULACTL?.trim();
  const candidates = configured
    ? [configured]
    : [...(env.PATH ?? '').split(path.delimiter).filter(Boolean).map((dir) => path.join(dir, 'aulactl')),
      path.join(env.HOME || os.homedir(), '.local', 'bin', 'aulactl')];
  return candidates.find((file) => path.isAbsolute(file) && isExecutable(file)) ?? null;
}

function isExecutable(file) {
  try { accessSync(file, constants.X_OK); return true; } catch { return false; }
}

const even = (value) => Math.max(2, Math.round(value / 2) * 2);

/** Overlay size for a video height and the size option (% of the height). */
export function overlaySize(videoHeight, sizePercent) {
  const height = even(videoHeight * sizePercent / 100);
  return { width: even(height * ASPECT), height };
}

export function available({ env }) {
  return findAulactl(env)
    ? { available: true }
    : { available: false, reason: 'aulactl not found (install it, or set AULACTL to its path)' };
}

const lastLine = (text) => text.trim().split(/\r?\n/).pop()?.slice(0, 300) || 'no output';

export async function videoStage(ctx) {
  if (!ctx.timeline) return { status: 'skipped', reason: `no timeline: ${ctx.timelineError ?? 'unknown'}` };
  const aulactl = findAulactl(ctx.tools.env);
  if (!aulactl) return { status: 'skipped', reason: 'aulactl not found' };
  if (!(ctx.video.durationMs > 0)) throw new Error('the video length is unknown');
  const replay = ctx.job.replayPath;
  // Find the session first: no matching recording is a quiet skip, not a failed encode.
  const check = await ctx.utils.runProcess(aulactl,
    ['replay', 'overlay', 'auto', '--osr', replay, '--json-only', '--out', path.join(ctx.workDir, 'aligned.json')]);
  if (check.code === NO_SESSION_EXIT) return { status: 'skipped', reason: 'no recorded keyboard session matches this replay' };
  if (check.code !== 0) throw new Error(`aulactl exited with ${check.code}: ${lastLine(check.output)}`);
  const size = { ...overlaySize(ctx.video.height, ctx.options.size), fps: FPS };
  const args = [
    'replay', 'overlay', 'auto', '--osr', replay, '--out', '-',
    '--width', String(size.width), '--height', String(size.height), '--fps', String(FPS),
    '--start-ms', ctx.timeline.startMs.toFixed(2), '--preroll-ms', ctx.timeline.prerollMs.toFixed(2),
    '--duration-ms', ctx.video.durationMs.toFixed(0),
  ];
  if (ctx.options.showDepth) args.push('--show-depth');
  const output = path.join(ctx.workDir, 'keys.mp4');
  const { encoder } = await ctx.utils.encodeVideo({
    inputs: ctx.utils.rawVideoInput(size),
    stdinFrom: { command: aulactl, args },
    filter: ctx.utils.overlayFilter({ input: '1:v', corner: ctx.options.corner, opacity: ctx.options.opacity }),
    output,
  });
  return {
    status: 'applied', output,
    note: `encoder ${encoder}, song ${ctx.timeline.startMs.toFixed(0)} ms at frame 0, pre-roll ${ctx.timeline.prerollMs.toFixed(0)} ms, rate ${ctx.timeline.rate}`,
  };
}
