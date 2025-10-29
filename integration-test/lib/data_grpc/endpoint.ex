defmodule DataGRPC.Endpoint do
  @moduledoc false

  use GRPC.Endpoint

  run(DataGRPC.Server)
end
