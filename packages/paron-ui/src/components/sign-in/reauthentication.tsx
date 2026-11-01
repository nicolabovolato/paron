import type {
  MutationFunction,
  UseMutationOptions,
} from "@tanstack/react-query";

import { useDialog } from "@/components/dialog";
import { isApiError } from "@/lib/api-error";

import { ReauthenticationDialog } from "./reauthentication-dialog";

export function useReauthentication() {
  const dialog = useDialog();

  return function withReauthentication<TData, TError, TVariables, TResult>(
    options: UseMutationOptions<TData, TError, TVariables, TResult>,
  ) {
    const { mutationFn } = options;
    if (!mutationFn) return options;

    const mutationFnWithReauthentication: MutationFunction<
      TData,
      TVariables
    > = async (variables, context) => {
      try {
        return await mutationFn(variables, context);
      } catch (error) {
        const challenged =
          isApiError(error) &&
          error.code === "insufficient_user_authentication";
        if (!challenged || !(await dialog.open(<ReauthenticationDialog />)))
          throw error;
        return mutationFn(variables, context);
      }
    };

    return { ...options, mutationFn: mutationFnWithReauthentication };
  };
}
