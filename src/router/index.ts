import { createRouter, createWebHashHistory } from "vue-router";
import Capture from "../views/Capture.vue";
import Editor from "../views/Editor.vue";
import Pin from "../views/Pin.vue";
import Settings from "../views/Settings.vue";

const routes = [
  { path: "/", component: Settings },
  { path: "/capture", component: Capture },
  { path: "/editor", component: Editor },
  { path: "/pin", component: Pin },
];

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

export default router;
