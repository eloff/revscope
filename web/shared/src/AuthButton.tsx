/// <reference types="solid-js" />

import { Show } from "solid-js";
import { useAuthState } from "./authState";
import LoggedInUser from "./LoggedInUser";
import LoginDialog from "./LoginDialog";

type AuthButtonProps = {
  triggerLabel?: string;
  triggerClass?: string;
};

export default function AuthButton(props: AuthButtonProps) {
  const { authUser, authResolved } = useAuthState();

  return (
    <Show when={authResolved()}>
      <Show
        when={authUser()}
        fallback={
          <LoginDialog triggerLabel={props.triggerLabel} triggerClass={props.triggerClass} />
        }
      >
        <LoggedInUser />
      </Show>
    </Show>
  );
}
