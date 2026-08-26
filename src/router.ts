import { ref } from "vue";

export type View = "startup" | "main";
export const currentView = ref<View>("startup");
export function navigate(v: View) {
  currentView.value = v;
}
