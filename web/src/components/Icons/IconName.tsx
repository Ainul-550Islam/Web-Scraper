// IconName — split from Icons.tsx; barrel re-exports keep every public path & name

import type { Icon } from "./Icon";

export type IconName = keyof typeof Icon;
