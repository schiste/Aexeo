import { handleAfterSave } from "../../../../src/plugin.js";

export default {
  hooks: {
    "content:afterSave": handleAfterSave,
  },
};
