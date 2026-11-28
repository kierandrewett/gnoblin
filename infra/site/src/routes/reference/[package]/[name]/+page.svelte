<script lang="ts">
    // A class or a service: its import line, who returns it, an example, every member at a glance, then the properties, methods and events,
    // what the other library adds, the errors and the types it names. Every spelled thing holds all three languages (reference.css shows one).
    import { codeBlocks } from "#lib/actions/code-blocks.ts";
    import ErrorTable from "#lib/components/reference/ErrorTable.svelte";
    import EventTable from "#lib/components/reference/EventTable.svelte";
    import ImportLine from "#lib/components/reference/ImportLine.svelte";
    import MethodList from "#lib/components/reference/MethodList.svelte";
    import PropertyTable from "#lib/components/reference/PropertyTable.svelte";
    import type { PageProps } from "./$types";

    let { data }: PageProps = $props();
    const c = $derived(data.page);
</script>

<svelte:head>
    <title>{c.titleText} · API reference · Gnoblin</title>
    <meta
        name="description"
        content="The {c.titleText} {c.kind}: its properties, methods and events in TypeScript, Python and Rust."
    />
</svelte:head>

<div use:codeBlocks={c}>
    <h1 class="ident">{@html c.titleHtml}</h1>
    <p class="ref-meta">
        {c.kind}{#if c.extendsHtml}
            · extends {@html c.extendsHtml}{/if} · since {c.since}
    </p>
    {#if c.ledeHtml}<p class="ref-lede">{@html c.ledeHtml}</p>{/if}
    {#if c.bodyHtml}<div class="ref-prose">{@html c.bodyHtml}</div>{/if}
    <ImportLine text={c.importText} />

    {#if c.createdBy || c.returnedBy.shown.length}
        <dl class="ref-facts">
            {#if c.createdBy}
                <dt>Created by</dt>
                <dd>{c.createdBy}</dd>
            {/if}
            {#if c.returnedBy.shown.length}
                <dt>Returned by</dt>
                <dd>
                    {#each c.returnedBy.shown as member (member.href)}<a href={member.href}>{@html member.html}</a
                        >{/each}
                    {#if c.returnedBy.more > 0}<span class="mute">and {c.returnedBy.more} more</span>{/if}
                </dd>
            {/if}
        </dl>
    {/if}

    {#if c.exampleHtml}
        <h2 id="sec-example">Example</h2>
        {@html c.exampleHtml}
    {/if}

    <h2 id="sec-members">Members</h2>
    <dl class="ref-glance">
        {#each c.glance as group (group.label)}
            <dt>{group.label}</dt>
            <dd>
                {#each group.links as link (link.href)}<a href={link.href}>{@html link.html}</a>{/each}
            </dd>
        {/each}
    </dl>

    {#if c.ctor}
        <h2 id="sec-constructor">Constructor</h2>
        <MethodList rows={[c.ctor]} />
    {/if}

    {#if c.properties.length}
        <h2 id="sec-properties">Properties <span class="cnt">{c.properties.length}</span></h2>
        <p class="ref-note">
            <span data-only="typescript"
                >Animated properties also have <code>set&lt;Name&gt;Timed(value, timing)</code>. Writing a property of
                an object that has ended throws <code>GnoblinError</code>.</span
            >
            <span data-only="python"
                >Each is a <code>@property</code>, with a setter when it is read/write. Animated properties also have
                <code>def set_&lt;name&gt;_timed(self, value, timing: Timing) -&gt; None</code>. Writing a property of
                an object that has ended raises <code>GnoblinError</code>.</span
            >
            <span data-only="rust"
                >Each property is a getter. A writable one has the setter on the line below it, and an animated one a
                timed setter. An object or record reads as <code>Option&lt;T&gt;</code>: it may be absent.</span
            >
        </p>
        <PropertyTable rows={c.properties} label="Properties of {c.titleText}" />
    {/if}

    {#if c.methods.length}
        <h2 id="sec-methods">Methods <span class="cnt">{c.methods.length}</span></h2>
        <MethodList rows={c.methods} />
    {/if}

    {#if c.events.length}
        <h2 id="sec-events">Events <span class="cnt">{c.eventCount}</span></h2>
        <p class="ref-note">
            <span data-only="typescript"
                >Listen with <code>object.on(event, handler)</code>, which returns a <code>Listener</code>. The handler
                receives the arguments listed here.</span
            >
            <span data-only="python"
                >Listen with <code>on(event, handler)</code>, or as a decorator, <code>@object.on(event)</code>. Both
                return a <code>Listener</code>. The handler receives the arguments listed here.</span
            >
            <span data-only="rust"
                >Each event is a method that takes a closure and returns a <code>Listener</code>. Dropping the listener
                stops it; <code>.keep()</code> keeps it.</span
            >
        </p>
        <EventTable rows={c.events} />
    {/if}

    {#if c.added}
        <h2 id="sec-added">
            Added by {c.added.package} <span class="cnt">{c.added.properties.length + c.added.methods.length}</span>
        </h2>
        <p class="ref-note">
            Present when <code>{c.added.package}</code> is loaded; a shell that loads only <code>gnoblin-core</code>
            does not have them. They are declared on {@html c.titleHtml} by <code>{c.added.package}</code>, so importing
            it is enough.
        </p>
        {#if c.added.properties.length}<PropertyTable
                rows={c.added.properties}
                label="Properties added by {c.added.package}"
            />{/if}
        {#if c.added.methods.length}<MethodList rows={c.added.methods} />{/if}
    {/if}

    {#if c.addedElsewhere.length}
        <h2 id="sec-elsewhere">Added to other classes <span class="cnt">{c.addedElsewhere.length}</span></h2>
        <p class="ref-note">
            This library adds these members to classes of <code>gnoblin-core</code>. They are listed on the page of the
            class they extend too, under <em>Added by</em>.
        </p>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="tbl-wrap" tabindex="0" role="region" aria-label="Members added to other classes">
            <table class="tbl ref-tbl">
                <thead>
                    <tr>
                        <th scope="col">Member</th>
                        <th scope="col">Type</th>
                        <th scope="col">Description</th>
                    </tr>
                </thead>
                <tbody>
                    {#each c.addedElsewhere as row (row.id)}
                        <tr>
                            <th scope="row"><a class="anchor" href={row.memberHref}>{@html row.memberHtml}</a></th>
                            <td data-label="Type">{@html row.typeHtml}</td>
                            <td data-label="Description">{@html row.descHtml}</td>
                        </tr>
                    {/each}
                </tbody>
            </table>
        </div>
    {/if}

    <h2 id="sec-errors">Errors</h2>
    <p class="ref-note">
        {#each Object.entries(c.errors.prose) as [lang, html] (lang)}<span data-only={lang}>{@html html}</span>{/each}
    </p>
    {#if c.errors.rows.length}
        <ErrorTable rows={c.errors.rows} />
    {:else}
        <p class="ref-note mute">
            The members of this class raise no error of their own. The codes every call can raise are listed under
            <a class="ty" href="/reference/errors">Errors</a>.
        </p>
    {/if}

    {#if c.types.length}
        <h2 id="sec-types">Types</h2>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="tbl-wrap" tabindex="0" role="region" aria-label="Types">
            <table class="tbl ref-tbl">
                <thead>
                    <tr>
                        <th scope="col">Type</th>
                        <th scope="col">Kind</th>
                        <th scope="col">Values and fields</th>
                    </tr>
                </thead>
                <tbody>
                    {#each c.types as row (row.id)}
                        <tr>
                            <th scope="row"><a class="ty nm" href={row.href}>{@html row.nameHtml}</a></th>
                            <td data-label="Kind">{row.kind}</td>
                            <td data-label="Values and fields">
                                <span class="vals">{@html row.valuesHtml}</span>
                                {#if row.descHtml}<span class="df">{@html row.descHtml}</span>{/if}
                            </td>
                        </tr>
                    {/each}
                </tbody>
            </table>
        </div>
    {/if}
</div>
