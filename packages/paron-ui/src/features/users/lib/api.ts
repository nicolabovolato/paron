import { useReauthentication } from "@/components/sign-in";
import {
  createUserMutation,
  listUsersOptions,
  readUserOptions,
  removeUserMutation,
  startRecoveryMutation,
  updateUserMutation,
  userCredentialsOptions,
} from "@/generated/api/@tanstack/react-query.gen";
import type { ListUsersData } from "@/generated/api/types.gen";
import { useMutation } from "@tanstack/react-query";

export const userQueries = {
  list: (query: ListUsersData["query"]) => listUsersOptions({ query }),
  detail: (id: string) => readUserOptions({ path: { id } }),
  credentials: (id: string) => userCredentialsOptions({ path: { id } }),
};

export function useCreateUser() {
  return useMutation(createUserMutation());
}

export function useUpdateUser() {
  return useMutation(updateUserMutation());
}

export function useRemoveUser() {
  return useMutation(removeUserMutation());
}

export function useStartRecovery() {
  const withReauthentication = useReauthentication();
  return useMutation(withReauthentication(startRecoveryMutation()));
}
