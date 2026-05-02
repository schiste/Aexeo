/* @ts-self-types="./aexeo_emdash_bridge.d.ts" */
import * as wasm from "./aexeo_emdash_bridge_bg.wasm";
import { __wbg_set_wasm } from "./aexeo_emdash_bridge_bg.js";

__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
export {
    evaluateDocuments, scoreIntelligence
} from "./aexeo_emdash_bridge_bg.js";
