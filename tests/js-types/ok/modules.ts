// Every module of the library a script names resolves, under the name the runtime loads it by.
import { InputContext } from "gnoblin-core/input";
import { Style } from "gnoblin-std/style";
import type { Reply } from "gnoblin-core/schema";
import type { Config } from "gnoblin-core/schema/gnoblin";
import { Box, Canvas } from "gnoblin-std/ui";

export { Box, Canvas, InputContext, Style };
export type Answer = Reply;
export type ConfigObject = Config;
