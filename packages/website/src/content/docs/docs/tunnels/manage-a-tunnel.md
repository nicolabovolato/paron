---
title: Manage a tunnel
description: Cancel a tunnel setup, revoke tunnel access, and restore it.
sidebar:
  order: 5
---

In the web UI, these actions are in the menu of the machine. You need the role Administrator or
Operator.

## Cancel tunnel setup

After you select **Set up tunnel**, the token is valid for 15 minutes. **Cancel tunnel setup** makes
the token invalid before that time. Use it if you gave the token to the wrong machine, if you do
not want the tunnel, or if you lost the token.

While a token is valid, **Set up tunnel** is not in the menu. To get a new token, cancel the
setup, then select **Set up tunnel** again.

## Revoke tunnel access

**Revoke tunnel access** removes the registration of the machine. Paron closes the connection of
the tunnel and the open terminals of the machine immediately. The tunnel cannot connect again, and
Paron cannot reach the machine until you [restore the access](#restore-tunnel-access).

## Restore tunnel access

After the tunnel access of a machine is revoked, its tunnel cannot connect. To restore the access,
give the machine a new token.

1. In the web UI, select **Set up tunnel** to get a new token.
2. On the machine, stop the tunnel. Only one `paron` process can use the data directory.
3. Run this command:

   ```sh
   paron tunnel enrol --data /var/lib/paron-tunnel --token 'paron://…'
   ```

4. Start the tunnel again.
