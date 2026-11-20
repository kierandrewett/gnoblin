/** The transforms an output takes: the compositor's names for a rotation and a flip. */
export type OutputTransform = "normal" | "90" | "180" | "270" | "flipped" | "flipped-90" | "flipped-180" | "flipped-270";
/**
 * What an output is told to do, as the argument of `Output.apply` and the settings of an `OutputProfile`. A key left
 * out keeps the output's value. The compositor applies it through its transactional engine and keeps it as the
 * output's profile, so the same monitor on the same port gets it back after an unplug. `primary` is not a setting: the
 * layout derives the primary output, and a request that names it throws.
 */
export interface OutputConfig {
    /** `"3440x1440@144"`, or `"preferred"` to keep what the backend drives. */
    mode?: string;
    /** From 0.5 to 4. */
    scale?: number;
    /**
     * Composes the output at this multiple of its mode (`mode * supersample` pixels, clients told
     * `scale * supersample`) and shows it at its mode, so the desktop keeps its logical size while screenshots have
     * higher resolution. Accepted values are from 1 to 16. Values 1, 2 and 4 are recommended; other values are
     * accepted when `mode * supersample` is a whole size, with a notice.
     */
    supersample?: number;
    /** Rotation and optional reflection applied to the output. */
    transform?: OutputTransform;
    /** The output's top-left corner in the one desktop coordinate space, `[x, y]`. Left out, it follows the row. */
    position?: readonly [number, number] | {
        readonly x: number;
        readonly y: number;
    };
    /** `false` switches the output off: it shows black and has no place in the desktop. */
    enabled?: boolean;
    /** The connector this output shows an exact copy of, letterboxed to fit its own panel. */
    mirrorOf?: string;
}
/**
 * The notice a supersample factor earns when it is not a recommended one, or null. The text is the compositor's own
 * (`gnoblin_output::supersample::notice`), so the log line and the script's diagnostic say the same thing.
 */
export declare function supersampleNotice(factor: number): string | null;
/**
 * The compositor's request text for `setting` (one `key=value` per line), checking each value and naming `path` and the
 * key in the error. `mirrorOf` is already a connector here; `Output.mirror` turns an `Output` into one.
 */
export declare function outputRequest(setting: Omit<OutputConfig, "mirrorOf"> & {
    mirrorOf?: string | null;
}, path: string): string;
/** What `outputRequest` takes: the keys of one output's request. */
export type OutputSetting = Parameters<typeof outputRequest>[0];
