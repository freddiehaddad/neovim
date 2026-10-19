#include <stddef.h>

#include "nvim/api/private/defs.h"
#include "nvim/api/private/dispatch.h"
#include "nvim/api/private/helpers.h"

#include "api/private/dispatch_wrappers.generated.h"

MsgpackRpcRequestHandler msgpack_rpc_get_handler_at(int hash)
{
  return method_handlers[hash];
}
