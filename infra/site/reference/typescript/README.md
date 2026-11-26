# Gnoblin TypeScript SDK

The two packages a TypeScript shell imports:

- **gnoblin-core** is the compositor's own objects: windows, outputs, input, the session and processes. A shell is complete on this alone.
- **gnoblin-std** is the desktop above it: workspaces, layouts, the scene, styles and widgets.

```typescript
import { gnoblin } from "gnoblin-core";
import { Workspace } from "gnoblin-std";
```

This reference is written from the declarations that Gnoblin installs for editors, so a member listed here is a member your editor offers. The descriptions are the comments of the schema files, the same as in the Python and Rust references.

The [guide to TypeScript shells](/docs/typescript-shell) explains how the pieces fit together.
