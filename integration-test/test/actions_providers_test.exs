defmodule ActionsProvidersTest do
  use ExUnit.Case

  setup_all do
    Test.Support.WasmServices.start()

    # start GRPC server to mock data-api
    children = [
      {GRPC.Server.Supervisor, endpoint: DataGRPC.Endpoint, port: 50054, start_server: true}
    ]

    opts = [strategy: :one_for_one, name: ActionsProviders]
    Supervisor.start_link(children, opts)

    :ok
  end

  test "calling wrong route redirect to example.com" do
    assert {:ok, %{status: 200, body: body}} = Tesla.get("http://localhost:8000/example")
    assert String.contains?(body, "Example Domain")
  end

  test "calling /data route succesfully" do
    assert {:ok, %{status: 200, body: body}} = Tesla.get("http://localhost:8000/data")
    assert %{"data" => %{}} == JSON.decode!(body)
  end

  test "when the data-rpc server return an error, the error message is returned correctly by the data-api provider" do
    assert {:ok, %{status: 500, body: body}} =
             Tesla.get("http://localhost:8000/data/06caae5da8234837a330c14a7350ed75")

    assert %{"errors" => [%{"message" => "something went wrong"}]} == JSON.decode!(body)
  end

  test "calling /data route return an error if jaws is invalid" do
    valid_config = Application.get_env(:jaws, :services)

    Application.put_env(:jaws, :services, services: [actions_js: [secret: "invalid secret"]])
    assert {:ok, %{status: 500, body: body}} = Tesla.get("http://localhost:8000/data")

    assert %{
             "errors" => [
               %{
                 "extensions" => %{"code" => "UNAUTHENTICATED"},
                 "message" => "Request not authenticated"
               }
             ]
           } ==
             JSON.decode!(body)

    Application.put_env(:jaws, :services, valid_config)
  end
end
