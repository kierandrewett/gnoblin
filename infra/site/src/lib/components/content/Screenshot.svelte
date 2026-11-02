<script lang="ts">
    // A real capture, shown plain: full frame, 1 px border, rounded corners, an optional caption. With `scheme`
    // the light capture shows in light mode and the dark one in dark mode. WebP at 1x and 2x with a PNG fallback.
    import shotsJson from "#lib/shots.json";

    /** Name -> [1x, 2x] sizes, written by scripts/build-shots.mjs. */
    const SHOTS: Record<string, { width: number; height: number }[]> = shotsJson;

    let {
        name,
        alt,
        caption = "",
        scheme = false,
        sizes = "(max-width: 760px) 100vw, 720px",
        eager = false,
    }: {
        /** Base name in static/shots, without -light/-dark when `scheme` is set. */
        name: string;
        alt: string;
        caption?: string;
        scheme?: boolean;
        sizes?: string;
        eager?: boolean;
    } = $props();

    const variants = $derived(
        (scheme ? (["light", "dark"] as const) : ([null] as const)).map((which) => {
            const file = which ? `${name}-${which}` : name;
            const dims = SHOTS[file];
            if (!dims || dims.length !== 2) throw new Error(`No screenshot ${file} in src/lib/shots.json`);
            const srcset = (ext: string) => dims.map((d) => `/shots/${file}-${d.width}.${ext} ${d.width}w`).join(", ");
            return {
                which,
                webp: srcset("webp"),
                png: srcset("png"),
                src: `/shots/${file}-${dims[0]!.width}.png`,
                ...dims[1]!,
            };
        }),
    );
</script>

<figure>
    <div class="shot">
        {#each variants as v (v.src)}
            <picture class={v.which ? `for-${v.which}` : undefined}>
                <source type="image/webp" srcset={v.webp} {sizes} />
                <img
                    src={v.src}
                    srcset={v.png}
                    {sizes}
                    {alt}
                    width={v.width}
                    height={v.height}
                    loading={eager && !v.which ? "eager" : "lazy"}
                    decoding="async"
                />
            </picture>
        {/each}
    </div>
    {#if caption}<figcaption class="shot-caption">{caption}</figcaption>{/if}
</figure>
