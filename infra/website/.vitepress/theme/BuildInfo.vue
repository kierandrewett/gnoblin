<script setup lang="ts">
// The build time and the commit that the site was built from. config.mts fills in both values at build time.
const sha = __BUILD_SHA__;
const builtAt = __BUILT_AT__;
const treeUrl = `https://github.com/kierandrewett/gnoblin/tree/${sha}`;
</script>

<template>
    <p class="build-info">
        Built <time :datetime="builtAt">{{ builtAt.replace("T", " ").replace(/:\d\d\.\d+Z$/, " UTC") }}</time> from
        <a :href="treeUrl" rel="noopener"
            ><code>{{ sha.slice(0, 8) }}</code></a
        >
    </p>
</template>
