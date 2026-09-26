@stage-R3
Feature: Physical-device attribute reads

  @id:PD-150
  Scenario: Attribute read with memory identity preset materializes runtime status and parsed bank fields
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the attribute-read transport trace should include DT8 content-DTR0 and bank 0/1 reads
    And adapter 0 physical device 0 eventually exposes the golden runtime status and memory-bank identity
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-195
  Scenario: Status bits 2 and 7 reach the API under their own §9.16 names
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually reports status flags lamp_on and power_cycle_seen
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-196
  Scenario: QUERY LIGHT SOURCE TYPE reaches the API as the byte the gear answered
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually reports light source type 6
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-156
  Scenario: Failed attribute read classifies the aborting attribute group in the operation result
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where the common_102 group aborts after contention retries
    When I start an attribute read for adapter 0 physical device 0 with attribute group "common_102" only
    Then the response status should be 202
    And the last operation eventually fails
    And the operation attribute-read outcomes should show "common_102" as "contended_abort" and "identity" as "success"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-155
  Scenario: Attribute read content-confirm recovers a contended physical minimum reply
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an attribute-read identity script where a contended physical minimum reply is corrected by content-confirm
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the attribute-read transport trace should retry the contended physical minimum query until content confirms
    And adapter 0 physical device 0 eventually exposes the golden runtime status and memory-bank identity
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-164
  Scenario: A device silent through the whole presence budget fails the read as device_absent
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 never answers the presence probe
    When I start an attribute read for adapter 0 physical device 0 with attribute group "groups" only
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "device_absent"
    And the operation attribute-read outcomes should show "identity" as "device_absent"
    And the operation attribute-read outcomes should show "groups" as "not_attempted"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-169
  Scenario: A device that vanishes mid-read is absent at the silence budget, not swept to the end
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 answers presence and then falls silent
    When I start an attribute read for adapter 0 physical device 0 with attribute group "groups" only
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "device_absent"
    And the operation attribute-read outcomes should show "identity" as "device_absent"
    And the operation attribute-read outcomes should show "groups" as "not_attempted"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-163
  Scenario: A bank that ends before its header says is published as a short read
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an attribute-read script where bank 0 ends before its header says
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the operation attribute-read outcomes should show "memory_banks" as "success"
    And adapter 0 physical device 0 eventually exposes bank 0 at the length the gear proved
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-165
  Scenario: An attribute read publishes the colour temperature it measured
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 answers 4000K as its active colour
    When I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes runtime colour temperature 4000 K
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-166
  Scenario: A colour the gear did not answer leaves the stored colour alone
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a cct 3000K target-state script for short address 0
    When I PUT JSON {"color_mode":"cct","color_temperature_kelvin":3000} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And adapter 0 physical device 0 eventually exposes runtime colour temperature 3000 K
    Given an attribute-read script where short address 0 leaves the colour temperature query unanswered
    When I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 still exposes runtime colour temperature 3000 K
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-167
  Scenario: An RGB-active gear has its dim levels read back into runtime state
    Given a six-channel DT8 discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 is RGB-active at 254 10 20
    When I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes runtime rgb 255 55 79
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-269
  Scenario: A power cycle the gear reports forgets the colour state it held in RAM
    Given a six-channel DT8 discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 is RGB-active at 254 10 20 and reports no power cycle
    When I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour
    Then the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes runtime rgb 255 55 79
    Given an attribute-read script with no device-type probe for short address 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "runtime_status" only
    Then the last operation eventually succeeds
    And adapter 0 physical device 0 eventually holds no colour state from before the power cycle
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-251
  Scenario: A colour the operator asked for reads back as the number they asked for
    Given a six-channel DT8 discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 is RGB-active at 254 116 26
    When I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes runtime rgb 255 180 90
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-168
  Scenario: A runtime-status read does not ask the gear what it is
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a cct 3000K target-state script for short address 0
    When I PUT JSON {"color_mode":"cct","color_temperature_kelvin":3000} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And adapter 0 physical device 0 eventually exposes runtime colour temperature 3000 K
    Given an attribute-read script with no device-type probe for short address 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "runtime_status" only
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 still exposes runtime colour temperature 3000 K
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-159
  Scenario: Doubled-byte group membership readback is re-read and healed
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a groups attribute-read script with a doubled-byte first pair for short address 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "groups" only
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes groups membership 2
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-170
  Scenario: An attribute read publishes the gear features byte it measured
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 answers gear features 0x41
    When I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes gear features 65
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-171
  Scenario: A gear that does not answer the features query invents no byte
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 leaves the gear features query unanswered
    When I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes runtime colour temperature 4000 K
    And adapter 0 physical device 0 exposes no gear features
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-178
  Scenario: An RGBWAF gear exposes its control byte and the channel capability
    Given a six-channel DT8 discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script where short address 0 is RGB-active at 254 10 20
    When I start an attribute read for adapter 0 physical device 0 with runtime status and dt8 colour
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 exposes dt8 attribute "rgbwaf_control" 128
    And adapter 0 physical device 0 reports capability "rgbwaf" true

  @id:PD-197
  Scenario: Memory bank 0 above 0x1A reaches the API as its own section
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes the bus unit configuration and implemented parts
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-198
  Scenario: A bank 1 declaring DiiA Part 251 content format 3 is parsed as luminaire data
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes the Part 251 luminaire data
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-199
  Scenario: A bank 1 with an unrecognised content format yields no luminaire data
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read script with an unrecognised bank 1 content format for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually reports the bank 1 content format without luminaire fields
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-256
  Scenario: A lamp-failure bit escalates a runtime read to the Part 207 failure byte
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script for a gear reporting lamp failure for short address 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "runtime_status" only
    Then the response status should be 202
    And the last operation eventually succeeds
    And the attribute-read transport trace should include the Part 207 failure query
    And adapter 0 physical device 0 eventually reports Part 207 failure byte 33
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-257
  Scenario: A healthy runtime read spends no Part 207 frames
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script with no device-type probe for short address 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "runtime_status" only
    Then the response status should be 202
    And the last operation eventually succeeds
    And the attribute-read transport trace should exclude the Part 207 failure query
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-258
  Scenario: A MASK actual level states no level and asks Part 207 why
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given an attribute-read script with no device-type probe for short address 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "runtime_status" only
    Then the last operation eventually succeeds
    And the physical device 0 state level should eventually be 127
    Given an attribute-read script where the gear answers MASK for its actual level
    When I start an attribute read for adapter 0 physical device 0 with attribute group "runtime_status" only
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually reports Part 207 failure byte 32
    And the physical device 0 state level should eventually be 127
    And all scripted DALI exchanges should be consumed without errors
