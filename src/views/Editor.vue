<template>
  <div class="editor-container">
    <img v-if="imageSrc" :src="imageSrc" draggable="false" />
    <div v-else class="placeholder">Loading image...</div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";

const imageSrc = ref("");

onMounted(() => {
  const params = new URLSearchParams(window.location.search);
  const path = params.get("path");
  if (path) {
    imageSrc.value = convertFileSrc(path);
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
