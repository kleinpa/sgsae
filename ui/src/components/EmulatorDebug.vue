<template>
  <div>
    <p>{{ generatedText }}</p>
    <button @click="generateText">Generate Text</button>
  </div>
</template>

<script setup>
import { ref } from 'vue'
import init, { run } from '../../app_bindgen'

// Declare a ref to store the generated text
const generatedText = ref('')

// Call the init function and handle text generation
const generateText = async () => {
  try {
    // Initialize the external resource (e.g., WASM)
    await init()

    // Call the run function and store the result
    const result = run()

    // Update the reactive state with the generated text
    generatedText.value = result
  } catch (error) {
    console.error('Failed to generate text:', error)
    generatedText.value = 'Error generating text!'
  }
}
</script>

<style scoped>
/* Add any necessary styles here */
button {
  margin-top: 10px;
}
</style>
