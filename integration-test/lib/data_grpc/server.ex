defmodule DataGRPC.Server do
  @moduledoc """
  The gRPC server. Used for integration test.
  Mimicking DataGRPC.Server of the DataAPI/DataRPC service.
  """

  use GRPC.Server, service: DataGRPC.DataAPI.Service

  alias DataGRPC.DataAPIRequest
  alias DataGRPC.DataAPIResult

  @spec execute(DataAPIRequest.t(), GRPC.Server.Stream.t()) :: DataAPIResult.t()
  def execute(%{context: %{application_id: application_id}} = _request, stream) do
    case authenticate(stream, application_id) do
      :ok ->
        case application_id do
          "06caae5da8234837a330c14a7350ed75" ->
            %DataAPIResult{
              status: :ERROR,
              result: format_error(%{"message" => "something went wrong"})
            }

          _ ->
            %DataAPIResult{status: :OK, result: format_result(%{})}
        end

      :unauthenticated ->
        %DataAPIResult{status: :ERROR, result: format_error(:unauthenticated)}
    end
  end

  @spec authenticate(GRPC.Server.Stream.t(), binary) :: :ok | :unauthenticated
  defp authenticate(stream, application_id) do
    token =
      stream
      |> GRPC.Stream.get_headers()
      |> Map.get("authorization", "")
      |> String.replace(~r/^Bearer\ /, "")

    case Jaws.verify(token) do
      {:ok, %{"application_id" => ^application_id}} -> :ok
      _ -> :unauthenticated
    end
  end

  defp format_result(result) do
    Jason.encode!(%{data: result})
  end

  defp format_error(:unauthenticated) do
    format_error(%{message: "Request not authenticated", extensions: %{code: "UNAUTHENTICATED"}})
  end

  defp format_error(error) do
    Jason.encode!(%{errors: List.wrap(error)})
  end
end
