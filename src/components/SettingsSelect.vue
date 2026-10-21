<template>
  <div ref="root" class="settings-select" :class="{ open }">
    <button
      ref="trigger"
      type="button"
      class="select-trigger"
      :aria-label="ariaLabel"
      :aria-expanded="open"
      aria-haspopup="listbox"
      @click="toggle"
      @keydown="onTriggerKeydown"
    >
      <span class="select-value">{{ selectedLabel }}</span>
      <svg class="chevron" viewBox="0 0 24 24" aria-hidden="true">
        <path d="m7 10 5 5 5-5" />
      </svg>
    </button>

    <Transition name="select-pop">
      <ul v-if="open" class="select-menu" role="listbox" :aria-label="ariaLabel">
        <li
          v-for="(option, index) in options"
          :key="option.value"
          role="option"
          :aria-selected="option.value === modelValue"
          :class="{
            selected: option.value === modelValue,
            active: index === activeIndex,
          }"
          @click="choose(option.value)"
          @mouseenter="activeIndex = index"
        >
          <span>{{ option.label }}</span>
          <svg v-if="option.value === modelValue" class="check" viewBox="0 0 24 24" aria-hidden="true">
            <path d="m5 12 4 4L19 6" />
          </svg>
        </li>
      </ul>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";

interface SelectOption {
  value: string;
  label: string;
}

const props = defineProps<{
  modelValue: string;
  options: SelectOption[];
  ariaLabel?: string;
}>();

const emit = defineEmits<{
  (event: "update:modelValue", value: string): void;
  (event: "change", value: string): void;
}>();

const root = ref<HTMLElement | null>(null);
const trigger = ref<HTMLButtonElement | null>(null);
const open = ref(false);
const activeIndex = ref(0);

const selectedLabel = computed(
  () => props.options.find((option) => option.value === props.modelValue)?.label ?? props.modelValue,
);

function selectedIndex(): number {
  const index = props.options.findIndex((option) => option.value === props.modelValue);
  return index < 0 ? 0 : index;
}

function toggle() {
  open.value = !open.value;
  if (open.value) {
    activeIndex.value = selectedIndex();
  }
}

function close() {
  open.value = false;
}

function choose(value: string) {
  emit("update:modelValue", value);
  emit("change", value);
  close();
  trigger.value?.focus();
}

function onTriggerKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    close();
    return;
  }

  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if (!open.value) {
      open.value = true;
      activeIndex.value = selectedIndex();
    } else {
      const delta = event.key === "ArrowDown" ? 1 : -1;
      activeIndex.value =
        (activeIndex.value + delta + props.options.length) % props.options.length;
    }
    return;
  }

  if ((event.key === "Enter" || event.key === " ") && open.value) {
    event.preventDefault();
    const option = props.options[activeIndex.value];
    if (option) choose(option.value);
  }
}

function onDocumentPointerDown(event: PointerEvent) {
  if (open.value && root.value && !root.value.contains(event.target as Node)) {
    close();
  }
}

onMounted(() => document.addEventListener("pointerdown", onDocumentPointerDown));
onBeforeUnmount(() => document.removeEventListener("pointerdown", onDocumentPointerDown));
</script>

<style scoped>
.settings-select {
  position: relative;
  min-width: 168px;
  flex: 0 0 auto;
}

.select-trigger {
  display: flex;
  width: 100%;
  min-height: 36px;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 7px 10px 7px 12px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 12.5px;
  text-align: left;
  cursor: pointer;
  outline: none;
  transition: border-color 0.16s ease, box-shadow 0.16s ease, background 0.16s ease;
}

.select-trigger:hover {
  border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
}

.select-trigger:focus-visible,
.settings-select.open .select-trigger {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.select-value {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chevron {
  width: 16px;
  height: 16px;
  flex: 0 0 16px;
  fill: none;
  stroke: var(--muted);
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.7;
  transition: transform 0.18s cubic-bezier(0.2, 0.8, 0.2, 1), stroke 0.16s ease;
}

.settings-select.open .chevron {
  transform: rotate(180deg);
  stroke: var(--accent);
}

.select-menu {
  position: absolute;
  z-index: 30;
  top: calc(100% + 6px);
  right: 0;
  left: 0;
  max-height: 230px;
  margin: 0;
  padding: 4px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--card);
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.14);
  list-style: none;
}

.select-menu li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  color: var(--text);
  font-size: 12.5px;
  line-height: 1.35;
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}

.select-menu li:hover,
.select-menu li.active {
  background: var(--accent-soft);
}

.select-menu li.selected {
  color: var(--accent);
  font-weight: 600;
}

.check {
  width: 14px;
  height: 14px;
  flex: 0 0 14px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 2;
}

.select-pop-enter-active,
.select-pop-leave-active {
  transition: opacity 0.14s ease, transform 0.14s cubic-bezier(0.2, 0.8, 0.2, 1);
}

.select-pop-enter-from,
.select-pop-leave-to {
  opacity: 0;
  transform: translateY(-4px) scale(0.985);
}

@media (prefers-reduced-motion: reduce) {
  .select-trigger,
  .chevron,
  .select-menu li,
  .select-pop-enter-active,
  .select-pop-leave-active {
    transition: none;
  }
}
</style>
