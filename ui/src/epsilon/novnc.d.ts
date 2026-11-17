// noVNC ships no types; EpsilonScreen.tsx only uses RFB's constructor, a few settings and its events.
declare module '@novnc/novnc' {
  const RFB: any;
  export default RFB;
}
