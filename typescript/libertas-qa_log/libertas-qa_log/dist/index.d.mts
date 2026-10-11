//#region src/index.d.ts
/** Reserved key schema; this sample does not persist records. */
type LogDataKey = {
  kind: 0;
} | {
  kind: 1;
};
/** Reserved stored values. */
type LogData = {
  kind: 0;
} | {
  kind: 1;
};
/** @LibertasDataSchema(key=LogDataKey,value=LogData) */
declare function test_log(): void;
//#endregion
export { LogData, LogDataKey, test_log };
//# sourceMappingURL=index.d.mts.map