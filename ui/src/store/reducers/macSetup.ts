import type {
  MacSetupEvent,
  MacSetupState,
  SetupStep,
  StepProgress,
  StepStatus,
} from "../../ipc/bindings";

/** Twin of `SetupStep::COMPONENTS`: what `Prepare` installs, in order. */
export const SETUP_COMPONENTS: readonly SetupStep[] = ["rosetta", "wine", "prefix", "connectivity"];

/** Every step the assistant lists, the game last. */
export const SETUP_STEPS: readonly SetupStep[] = [...SETUP_COMPONENTS, "game"];

const NO_PROGRESS: StepProgress = { step: null, permille: null, doneMb: 0, totalMb: 0 };

const STEP_FIELD = {
  rosetta: "rosetta",
  wine: "wine",
  prefix: "prefix",
  connectivity: "connectivity",
  game: "game",
} as const satisfies Record<SetupStep, keyof MacSetupState>;

/** Twin of `MacSetupState::status`. */
export function stepStatus(state: MacSetupState, step: SetupStep): StepStatus {
  return state[STEP_FIELD[step]];
}

/** Twin of `MacSetupState::components_ready`. */
export function componentsReady(state: MacSetupState): boolean {
  return SETUP_COMPONENTS.every((step) => stepStatus(state, step).type === "ready");
}

/** Twin of `MacSetupState::complete`. */
export function setupComplete(state: MacSetupState): boolean {
  return componentsReady(state) && state.game.type === "ready";
}

/** Twin of `MacSetupState::busy`. */
export function setupBusy(state: MacSetupState): boolean {
  return state.active !== null || state.steam.type !== "idle";
}

function withStatus(state: MacSetupState, step: SetupStep, status: StepStatus): MacSetupState {
  return { ...state, [STEP_FIELD[step]]: status };
}

function settle(state: MacSetupState, step: SetupStep, status: StepStatus): MacSetupState {
  const next = withStatus(state, step, status);
  return state.active === step ? { ...next, active: null, progress: NO_PROGRESS } : next;
}

export function reduceMacSetup(state: MacSetupState, event: MacSetupEvent): MacSetupState {
  switch (event.type) {
    case "probed": {
      const probe = event.payload.probe;
      let next: MacSetupState = {
        ...state,
        supported: probe.supported,
        checked: true,
        rootDir: probe.rootDir,
        gameDir: probe.gameDir,
        freeMb: probe.freeMb,
        steamUser: probe.steamUser !== "" ? probe.steamUser : state.steamUser,
      };
      const present: Record<SetupStep, boolean> = {
        rosetta: probe.rosetta,
        wine: probe.wine,
        prefix: probe.prefix,
        connectivity: probe.connectivity,
        game: probe.game,
      };
      for (const step of SETUP_STEPS) {
        const current = stepStatus(next, step);
        if (current.type === "running") continue;
        if (present[step]) next = withStatus(next, step, { type: "ready" });
        else if (current.type !== "failed") next = withStatus(next, step, { type: "missing" });
      }
      return next;
    }
    case "stepStarted":
      return {
        ...withStatus(state, event.payload.step, { type: "running" }),
        active: event.payload.step,
        progress: { ...NO_PROGRESS, step: event.payload.step },
      };
    case "progressed":
      return { ...state, progress: event.payload.progress };
    case "stepFinished":
      return settle(state, event.payload.step, { type: "ready" });
    case "stepFailed":
      return settle(state, event.payload.step, {
        type: "failed",
        payload: { reason: event.payload.reason },
      });
    case "stepCancelled":
      return settle(state, event.payload.step, { type: "missing" });
    case "steamPhaseChanged":
      return { ...state, steam: event.payload.phase };
    case "assistantShown":
      return { ...state, assistantOpen: true };
    case "assistantHidden":
      return { ...state, assistantOpen: false };
  }
}
