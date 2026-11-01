import { errorToast } from "@/components/error-toast";
import { useToastManager } from "@/components/shadcn/toast";
import {
  useIsTestingMachineConnection,
  useTestMachineConnection,
  useTestNewMachineConnection,
} from "@/features/machines/lib/api";
import type { Options } from "@/generated/api/sdk.gen";
import type {
  TestMachineConnectionData,
  TestNewMachineConnectionData,
} from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { isApiError, type ApiError } from "@/lib/api-error";

export function useConnectionTest(machineId: string | undefined) {
  const toast = useToastManager();
  const testSaved = useTestMachineConnection();
  const testUnsaved = useTestNewMachineConnection();
  const testingSaved = useIsTestingMachineConnection(machineId);

  const run = async (name: string, request: Promise<unknown>) => {
    try {
      await toast.promise(request, {
        loading: {
          title: m.connection_test_loading_title(),
          description: m.connection_test_loading_description({ name }),
        },
        success: {
          title: m.connection_test_success_title(),
          description: m.connection_test_success_description({ name }),
        },
        error: (error: ApiError) =>
          errorToast(m.connection_test_error(), error),
      });
      return { ok: true } as const;
    } catch (error) {
      if (!isApiError(error)) throw error;
      return { ok: false, error } as const;
    }
  };

  return {
    pending: machineId === undefined ? testUnsaved.isPending : testingSaved,
    saved: (name: string, options: Options<TestMachineConnectionData>) =>
      run(name, testSaved.mutateAsync(options)),
    unsaved: (name: string, options: Options<TestNewMachineConnectionData>) =>
      run(name, testUnsaved.mutateAsync(options)),
  };
}
