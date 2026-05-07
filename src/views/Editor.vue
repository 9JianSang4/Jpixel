<template>
  <div class="editor-container">
    <img v-if="imageSrc" :src="imageSrc" draggable="false" />
    <div v-else class="placeholder">Loading image...</div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRoute } from "vue-router";
import { invoke } from "@tauri-apps/api/core";

const imageSrc = ref("");
const route = useRoute();

onMounted(async () => {
  const path = route.query.path as string | undefined;
  if (path) {
    try {
      const base64: string = await invoke("read_image_base64", { path });
      imageSrc.value = base64;
    } catch (e) {
      console.error("Failed to load image:", e);
    }
  }
});
</script>

<style scoped>
.editor-container {
  width: 100vw;
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(20, 20, 25, 0.92);
  overflow: auto;
}

.editor-container img {
  max-width: 95vw;
  max-height: 95vh;
  object-fit: contain;
  box-shadow: 0 8px 40px rgba(0, 0, 0, 0.5);
  border-radius: 4px;
}

.placeholder {
  color: rgba(255, 255, 255, 0.6);
  font-size: 16px;
}
</style>
