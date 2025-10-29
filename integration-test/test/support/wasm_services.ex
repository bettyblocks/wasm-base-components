defmodule Test.Support.WasmServices do
  @moduledoc """
  Will deploy the wadm file to wasm.
  Also, when running locally will start test docker container for nats, wasm_cloud and wadm
  If starting test containers fails, try to stop other docker containers you have running.
  """

  use ExUnit.Case
  @yaml_file YamlElixir.read_from_file!("wadm.test.yaml")
  @files_to_copy [
    "components/fetcher/build/http_hello_world_s.wasm",
    "../providers/data-api/build/data-api.par.gz",
    "../providers/key-vault/build/key-vault.par.gz"
  ]
  def start() do
    if in_ci?() do
      deploy_app()
    else
      container = start_containers()
      :ok = deploy_app()
      container
    end
  end

  def start_containers() do
    address = Application.get_env(:wadm_client, :nats_host, "127.0.0.1")
    port = Application.get_env(:wadm_client, :nats_port, 4222)

    start_container(
      Testcontainers.Container.new("nats:2.10-alpine")
      |> Testcontainers.Container.with_cmd(["-js"])
      |> Testcontainers.Container.with_exposed_port(4222)
      |> Testcontainers.Container.with_waiting_strategy(
        Testcontainers.LogWaitStrategy.new(~r"Listening for client connections")
      )
      |> Testcontainers.Container.with_network_mode("host")
    )

    container =
      start_container(
        Testcontainers.Container.new("wasmcloud/wasmcloud:1.7.1-wolfi")
        |> Testcontainers.Container.with_environment("WASMCLOUD_ALLOW_FILE_LOAD", "true")
        |> Testcontainers.Container.with_environment("WASMCLOUD_RPC_HOST", address)
        |> Testcontainers.Container.with_environment("WASMCLOUD_RPC_PORT", "#{port}")
        |> Testcontainers.Container.with_environment("WASMCLOUD_CTL_HOST", address)
        |> Testcontainers.Container.with_environment("WASMCLOUD_CTL_PORT", "#{port}")
        |> Testcontainers.Container.with_network_mode("host")
        |> Testcontainers.Container.with_waiting_strategy(
          Testcontainers.LogWaitStrategy.new(~r"wasmCloud host started")
        )
      )

    copy_files_in_container(container)

    start_container(
      Testcontainers.Container.new("ghcr.io/wasmcloud/wadm:0.21.0-wolfi")
      |> Testcontainers.Container.with_environment("WADM_NATS_SERVER", "#{address}:#{port}")
      |> Testcontainers.Container.with_network_mode("host")
    )
  end

  defp copy_files_in_container(container, try_building? \\ true) do
    if Enum.all?(@files_to_copy, &File.exists?(&1)) do
      Enum.each(@files_to_copy, &copy_file_in_container(&1, container.container_id))
    else
      try_building_missing_files(container, try_building?)
    end
  end

  defp copy_file_in_container(file_name, container_id) do
    System.cmd("docker", ["container", "cp", file_name, "#{container_id}:/tmp"])
  end

  defp try_building_missing_files(container, true) do
    System.cmd("just", ["build"])
    copy_files_in_container(container, false)
  end

  defp try_building_missing_files(_, false), do: raise("missing build files")

  defp deploy_app do
    address = Application.get_env(:wadm_client, :nats_host, "127.0.0.1")
    port = Application.get_env(:wadm_client, :nats_port, 4222)

    {:ok, gnat} = Gnat.start_link(%{host: address, port: port, no_responders: true})

    on_exit(fn -> Gnat.stop(gnat) end)

    deploy_to_wadm(gnat)
  end

  defp start_container(config) do
    {:ok, container} = Testcontainers.start_container(config)
    ExUnit.Callbacks.on_exit(fn -> Testcontainers.stop_container(container.container_id) end)
    container
  end

  defp deploy_to_wadm(gnat) do
    conn = WadmClient.from_gnat(gnat, "default", nil)

    wait_until_ready(conn)
    yaml = resolve_relative_path_to_absolute_path(@yaml_file)
    app_name = yaml["metadata"]["name"]

    on_exit(fn -> WadmClient.delete_manifest(conn, app_name) end)

    {:ok, []} = WadmClient.list_manifests(conn)

    {:ok, %{"result" => "created"}} = WadmClient.put_manifest(conn, yaml)

    {:ok, [%{"name" => ^app_name, "status" => "undeployed"}]} =
      WadmClient.list_manifests(conn)

    {:ok, %{"result" => "acknowledged"}} = WadmClient.deploy_manifest(conn, app_name)

    {:ok,
     %{
       "result" => "success",
       "versions" => [%{"deployed" => true, "version" => _}]
     }} = WadmClient.list_versions(conn, app_name)

    :ok = wait_until_deployed(conn, app_name)
  end

  defp resolve_relative_path_to_absolute_path(yaml) when is_map(yaml) do
    Map.new(yaml, fn {key, value} -> {key, resolve_relative_path_to_absolute_path(value)} end)
  end

  defp resolve_relative_path_to_absolute_path(value) when is_list(value) do
    Enum.map(value, &resolve_relative_path_to_absolute_path(&1))
  end

  defp resolve_relative_path_to_absolute_path("file://./" <> relative_path) do
    if in_ci?() do
      "file://#{Path.expand(relative_path)}"
    else
      "file:///tmp/#{Path.basename(relative_path)}"
    end
  end

  defp resolve_relative_path_to_absolute_path(value) do
    value
  end

  defp wait_until_ready(conn) do
    # there is some weirdness that there are no_responders, just try until there are.
    0..100
    |> Enum.reduce_while(:error, fn _, _ ->
      case WadmClient.list_manifests(conn) do
        {:ok, _} -> {:halt, :ok}
        {:error, _} -> {:cont, :error}
      end
    end)
  end

  defp wait_until_deployed(conn, name) do
    0..100
    |> Enum.reduce_while(:error, fn _, _ ->
      case WadmClient.get_manifest_status(conn, name) do
        {:ok, %{"status" => %{"status" => %{"type" => status}}}}
        when status in ["undeployed", "reconciling"] ->
          Process.sleep(1000)
          {:cont, :error}

        {:ok, %{"status" => %{"status" => %{"type" => "deployed"}}}} ->
          {:halt, :ok}

        error ->
          {:halt, error}
      end
    end)
    |> case do
      :ok -> Process.sleep(100)
      other -> other
    end
  end

  def in_ci? do
    not is_nil(System.get_env("CI", nil))
  end
end
