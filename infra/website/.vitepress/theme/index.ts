import DefaultTheme from "vitepress/theme";
import { h } from "vue";
import BuildInfo from "./BuildInfo.vue";
import "./custom.css";

export default {
    extends: DefaultTheme,
    // layout-bottom puts the build information at the end of every page.
    Layout: () => h(DefaultTheme.Layout, null, { "layout-bottom": () => h(BuildInfo) }),
};
