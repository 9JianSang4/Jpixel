<template>
  <div class="pin-container">
    <img v-if="imageSrc" :src="imageSrc" draggable="false" />
    <div v-else class="placeholder">Pin Window</div>
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";
const imageSrc = ref("");
</script>

<style scoped>
.pin-container {
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  background: transparent;
}
.pin-container img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.placeholder {
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
}
</style>
