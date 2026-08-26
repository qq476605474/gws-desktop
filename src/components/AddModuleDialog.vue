<script setup lang="ts">
import { ref } from "vue";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";

const props = defineProps<{ wsPath: string }>();
const emit = defineEmits<{ (e: "close"): void; (e: "added"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();
const selected = ref<string[]>([]);
const submitting = ref(false);
const err = ref("");
const progress = ref("");

async function add() {
  if (submitting.value) return;
  submitting.value = true;
  err.value = "";
  try {
    const total = selected.value.length;
    for (let i = 0; i < total; i++) {
      const m = selected.value[i];
      progress.value = `正在添加 ${m}（${i + 1}/${total}）…`;
      const run = await cmd.exec(`gws add ${m}`, ["add", m], props.wsPath);
      await cmd.waitDone(run); // 逐个等结束，避免并发 worktree 操作
    }
    emit("added");
    emit("close");
  } catch (e) {
    err.value = String(e);
    submitting.value = false;
  }
}
</script>

<template>
  <div class="mask" @click.self="!submitting && emit('close')">
    <div class="dialog">
      <h3>增加模块</h3>
      <label v-for="r in hub.repos" :key="r.name">
        <input type="checkbox" :value="r.name" v-model="selected" :disabled="submitting" /> {{ r.name }}
      </label>
      <p v-if="progress" class="progress">{{ progress }}</p>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button :disabled="submitting" @click="emit('close')">取消</button>
        <button class="primary" :disabled="!selected.length || submitting" @click="add">添加</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: rgba(0,0,0,.35); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: #fff; border-radius: 8px; padding: 20px; width: 380px; display: flex; flex-direction: column; gap: 8px; max-height: 80vh; overflow: auto; }
label { display: flex; align-items: center; gap: 8px; font-size: 13px; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.primary { background: #1565c0; color: #fff; }
.progress { color: #888; font-size: 13px; margin: 0; }
.err { color: #c62828; font-size: 13px; margin: 0; }
</style>
