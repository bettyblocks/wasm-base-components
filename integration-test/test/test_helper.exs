if not Test.Support.WasmServices.in_ci?() do
  Testcontainers.start_link()
end

ExUnit.start()
