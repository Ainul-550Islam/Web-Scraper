//! Browser authoring feature barrel and route-integrated exports.
export { default as BrowserStudio } from "./BrowserStudio";
export { default as ElementPicker } from "./ElementPicker";
export { default as SelectorEditor } from "./SelectorEditor";
export { default as SelectorTree, findNode } from "./SelectorTree";
export { default as PreviewTable } from "./PreviewTable";
export { default as InteractionBuilder } from "./InteractionBuilder";
export type { ElementChoice } from "./ElementPicker";
export type { SelectorField, ExtractionKind } from "./SelectorEditor";
export type { StudioAction } from "./InteractionBuilder";
